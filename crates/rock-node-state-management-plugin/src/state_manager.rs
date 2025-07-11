use anyhow::{anyhow, Context, Result};
use prost::Message;
use rock_node_core::{
    block_reader::BlockReader,
    database::{DatabaseManager, CF_METADATA, CF_STATE_DATA, STATE_LAST_PROCESSED_BLOCK},
    events::BlockPersisted,
    state_reader::StateReader,
    BlockDataCache,
};
use rock_node_protobufs::com::hedera::hapi::block::stream::{
    block_item::Item as BlockItemType,
    output::{state_change::ChangeOperation, MapChangeKey, StateChange},
    Block,
};
use std::sync::Arc;
use tracing::{instrument, warn};

/// Manages the application of state changes to the database and provides
/// a read-only interface to the resulting state.
#[derive(Debug, Clone)]
pub struct StateManager {
    db_manager: Arc<DatabaseManager>,
    cache: Arc<BlockDataCache>,
    // Add a handle to the block reader for the catch-up logic.
    block_reader: Arc<dyn BlockReader>,
}

impl StateManager {
    /// Creates a new `StateManager`.
    pub fn new(
        db_manager: Arc<DatabaseManager>,
        cache: Arc<BlockDataCache>,
        block_reader: Arc<dyn BlockReader>,
    ) -> Self {
        Self {
            db_manager,
            cache,
            block_reader,
        }
    }

    /// Retrieves the last block number that was successfully processed by this plugin.
    pub fn get_last_processed_block(&self) -> Result<Option<u64>> {
        let db = self.db_manager.db_handle();
        let cf_metadata = db
            .cf_handle(CF_METADATA)
            .context("Failed to get CF_METADATA handle")?;
        Ok(db
            .get_cf(&cf_metadata, STATE_LAST_PROCESSED_BLOCK)?
            .map(|v| u64::from_be_bytes(v.try_into().unwrap())))
    }

    /// Processes a block coming from the live event stream (and its cache entry).
    #[instrument(skip_all, fields(block_number = event.block_number))]
    pub async fn apply_state_from_block_event(&self, event: BlockPersisted) -> Result<()> {
        let block_bytes = self
            .cache
            .get(&event.cache_key)
            .map(|d| d.contents)
            .ok_or_else(|| anyhow!("Block data not found in cache for key {}", event.cache_key))?;

        self.apply_state_changes(event.block_number, &block_bytes)
            .await?;

        // Only mark for removal if we consumed from the cache.
        self.cache.mark_for_removal(event.cache_key).await;

        Ok(())
    }

    /// Processes a block fetched directly from storage during a catch-up.
    #[instrument(skip_all, fields(block_number))]
    pub async fn apply_state_from_storage(&self, block_number: u64) -> Result<()> {
        let block_bytes = self.block_reader.read_block(block_number)?.ok_or_else(|| {
            anyhow!(
                "Block #{} not found in persistence for catch-up.",
                block_number
            )
        })?;

        self.apply_state_changes(block_number, &block_bytes).await
    }

    /// The generic, core logic for applying state changes from a block's raw bytes.
    async fn apply_state_changes(&self, block_number: u64, block_bytes: &[u8]) -> Result<()> {
        let block = Block::decode(block_bytes).context("Failed to deserialize Block protobuf")?;

        let db = self.db_manager.db_handle();
        let cf_state = db
            .cf_handle(CF_STATE_DATA)
            .context("CF_STATE_DATA not found")?;
        let cf_metadata = db.cf_handle(CF_METADATA).context("CF_METADATA not found")?;

        let mut batch = rocksdb::WriteBatch::default();
        for item in block.items {
            if let Some(BlockItemType::StateChanges(set)) = item.item {
                for change in set.state_changes {
                    self.apply_single_state_change(&mut batch, &cf_state, change)?;
                }
            }
        }

        batch.put_cf(
            &cf_metadata,
            STATE_LAST_PROCESSED_BLOCK,
            &block_number.to_be_bytes(),
        );
        db.write(batch).context("Failed to write state batch")
    }

    /// Dispatches a single `StateChange` operation to the RocksDB `WriteBatch`.
    fn apply_single_state_change(
        &self,
        batch: &mut rocksdb::WriteBatch,
        cf: &rocksdb::ColumnFamily,
        change: StateChange,
    ) -> Result<()> {
        if let Some(op) = change.change_operation {
            match op {
                ChangeOperation::MapUpdate(update) => {
                    let key = self.construct_db_key(change.state_id, &update.key)?;
                    let val = update.value.context("MapUpdateChange is missing value")?;
                    batch.put_cf(cf, &key, &val.encode_to_vec());
                }
                ChangeOperation::MapDelete(deletion) => {
                    let key = self.construct_db_key(change.state_id, &deletion.key)?;
                    batch.delete_cf(cf, &key);
                }
                _ => warn!("Unhandled StateChange operation: {:?}", op),
            }
        }
        Ok(())
    }

    /// Creates a composite database key from the state ID and the specific entity key.
    fn construct_db_key(&self, state_id: u32, map_key: &Option<MapChangeKey>) -> Result<Vec<u8>> {
        let entity_key_bytes = map_key
            .as_ref()
            .map(|k| k.encode_to_vec())
            .context("MapChangeKey is missing")?;
        Ok([state_id.to_be_bytes().as_slice(), &entity_key_bytes].concat())
    }
}

impl StateReader for StateManager {
    #[instrument(skip(self), fields(key_len = key.len()))]
    fn get_state_value(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let db = self.db_manager.db_handle();
        let cf = db
            .cf_handle(CF_STATE_DATA)
            .context("CF_STATE_DATA not found")?;
        Ok(db.get_cf(&cf, key)?)
    }
}
