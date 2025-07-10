use anyhow::{Context, Result};
use prost::Message;
use rock_node_core::{
    database::{DatabaseManager, CF_METADATA, CF_STATE_DATA, STATE_LAST_PROCESSED_BLOCK},
    events::BlockPersisted,
    state_reader::StateReader,
    BlockDataCache,
};
// Corrected Protobuf import paths based on your successful fix.
use rock_node_protobufs::com::hedera::hapi::block::stream::{
    block_item::Item as BlockItemType,
    output::{state_change::ChangeOperation, MapChangeKey, StateChange},
    Block,
};
use std::sync::Arc;
use tracing::{debug, error, instrument, warn};

/// Manages the application of state changes to the database and provides
/// a read-only interface to the resulting state.
#[derive(Debug, Clone)]
pub struct StateManager {
    db_manager: Arc<DatabaseManager>,
    cache: Arc<BlockDataCache>,
}

impl StateManager {
    /// Creates a new `StateManager`.
    pub fn new(db_manager: Arc<DatabaseManager>, cache: Arc<BlockDataCache>) -> Self {
        Self { db_manager, cache }
    }

    /// Retrieves the last block number that was successfully processed by this plugin.
    pub fn get_last_processed_block(&self) -> Result<Option<u64>> {
        let db = self.db_manager.db_handle();
        let cf_metadata = db
            .cf_handle(CF_METADATA)
            .context("Failed to get CF_METADATA handle")?;

        let value = db
            .get_cf(&cf_metadata, STATE_LAST_PROCESSED_BLOCK)?
            .map(|db_vec| {
                let mut bytes = [0u8; 8];
                bytes.copy_from_slice(&db_vec);
                u64::from_be_bytes(bytes)
            });

        Ok(value)
    }

    /// The core transaction for processing a persisted block.
    #[instrument(skip_all, fields(block_number = event.block_number))]
    pub fn apply_state_from_block(&self, event: BlockPersisted) -> Result<()> {
        let block_data = self.cache.get(&event.cache_key).ok_or_else(|| {
            anyhow::anyhow!("Block data not found in cache for key {}", event.cache_key)
        })?;

        let block =
            Block::decode(&*block_data.contents).context("Failed to deserialize Block protobuf")?;

        let db = self.db_manager.db_handle();
        let cf_state = db
            .cf_handle(CF_STATE_DATA)
            .context("Failed to get CF_STATE_DATA handle")?;
        let cf_metadata = db
            .cf_handle(CF_METADATA)
            .context("Failed to get CF_METADATA handle")?;

        let mut batch = rocksdb::WriteBatch::default();
        let mut changes_found = 0;

        for block_item in block.items {
            if let Some(BlockItemType::StateChanges(state_changes_set)) = block_item.item {
                for state_change in state_changes_set.state_changes {
                    // Pass the ColumnFamily handle by reference.
                    self.apply_single_state_change(&mut batch, &cf_state, state_change)?;
                    changes_found += 1;
                }
            }
        }

        if changes_found > 0 {
            debug!(
                "Extracted {} state changes for block {}.",
                changes_found, event.block_number
            );
        }

        batch.put_cf(
            &cf_metadata, // Pass the handle by reference
            STATE_LAST_PROCESSED_BLOCK,
            &event.block_number.to_be_bytes(),
        );

        db.write(batch)
            .context("Failed to write state batch to RocksDB")?;

        self.cache.remove(&event.cache_key);

        Ok(())
    }

    /// Dispatches a single `StateChange` operation to the RocksDB `WriteBatch`.
    fn apply_single_state_change(
        &self,
        batch: &mut rocksdb::WriteBatch,
        cf: &rocksdb::ColumnFamily, // Corrected Type: Direct reference
        change: StateChange,
    ) -> Result<()> {
        if let Some(operation) = change.change_operation {
            match operation {
                ChangeOperation::MapUpdate(update) => {
                    let key = self
                        .construct_db_key(change.state_id, &update.key)
                        .context("Failed to construct DB key for MapUpdate")?;
                    let value = update.value.context("MapUpdateChange is missing a value")?;
                    let value_bytes = value.encode_to_vec();
                    batch.put_cf(cf, &key, &value_bytes);
                }
                ChangeOperation::MapDelete(deletion) => {
                    let key = self
                        .construct_db_key(change.state_id, &deletion.key)
                        .context("Failed to construct DB key for MapDelete")?;
                    batch.delete_cf(cf, &key);
                }
                _ => {
                    warn!(
                        "Unhandled StateChange operation type: {:?}",
                        std::any::type_name_of_val(&operation)
                    );
                }
            }
        }
        Ok(())
    }

    /// Creates a composite database key from the state ID and the specific entity key.
    fn construct_db_key(&self, state_id: u32, map_key: &Option<MapChangeKey>) -> Result<Vec<u8>> {
        // FIX: Serialize the entire MapChangeKey struct, not just the oneof part.
        let entity_key_bytes = map_key
            .as_ref()
            .map(|k| k.encode_to_vec())
            .context("MapChangeKey is missing")?;

        let mut final_key = state_id.to_be_bytes().to_vec();
        final_key.extend(entity_key_bytes);

        Ok(final_key)
    }
}

// Implement the public read-only trait for the StateManager.
impl StateReader for StateManager {
    #[instrument(skip(self), fields(key_len = key.len()))]
    fn get_state_value(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let db = self.db_manager.db_handle();
        let cf = db
            .cf_handle(CF_STATE_DATA)
            .context("Failed to get CF_STATE_DATA handle for read")?;

        match db.get_cf(&cf, key) {
            // Pass by reference
            Ok(Some(value)) => {
                debug!("State key found.");
                Ok(Some(value))
            }
            Ok(None) => {
                debug!("State key not found.");
                Ok(None)
            }
            Err(e) => {
                error!("Failed to read from state database: {}", e);
                Err(e.into())
            }
        }
    }
}
