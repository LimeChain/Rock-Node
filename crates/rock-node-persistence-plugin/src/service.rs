use crate::{
    cold_storage::{archiver::Archiver, reader::ColdReader},
    hot_tier::HotTier,
    state::StateManager,
};
use anyhow::{anyhow, Result};
use rock_node_core::{block_reader::BlockReader, block_writer::BlockWriter};
use rock_node_protobufs::com::hedera::hapi::block::stream::{block_item, Block};
use rocksdb::WriteBatch;
use std::sync::Arc;
use tracing::{info, warn};

#[derive(Debug, Clone)]
pub struct PersistenceService {
    hot_tier: Arc<HotTier>,
    cold_reader: Arc<ColdReader>,
    archiver: Arc<Archiver>,
    state: Arc<StateManager>,
}

impl PersistenceService {
    pub fn new(
        hot_tier: Arc<HotTier>,
        cold_reader: Arc<ColdReader>,
        archiver: Arc<Archiver>,
        state: Arc<StateManager>,
    ) -> Self {
        Self {
            hot_tier,
            cold_reader,
            archiver,
            state,
        }
    }
}

impl BlockReader for PersistenceService {
    fn get_latest_persisted_block_number(&self) -> Result<Option<u64>> {
        self.state.get_latest_persisted()
    }

    fn get_earliest_persisted_block_number(&self) -> Result<Option<u64>> {
        self.state.get_true_earliest_persisted()
    }

    fn read_block(&self, block_number: u64) -> Result<Option<Vec<u8>>> {
        if let Some(block_bytes) = self.hot_tier.read_block(block_number)? {
            return Ok(Some(block_bytes));
        }
        self.cold_reader.read_block(block_number)
    }

    fn get_highest_contiguous_block_number(&self) -> Result<u64> {
        self.state.get_highest_contiguous()
    }
}

impl BlockWriter for PersistenceService {
    fn write_block(&self, block: &Block) -> Result<()> {
        let block_number = get_block_number(block)?;
        let mut batch = WriteBatch::default();

        if self.state.get_true_earliest_persisted()?.is_none() {
             self.state.set_true_earliest_persisted(block_number, &mut batch)?;
        }

        let is_contiguous = match self.get_highest_contiguous_block_number() {
            Ok(0) if self.get_latest_persisted_block_number()?.is_none() => true,
            Ok(current_contiguous) => block_number == current_contiguous + 1,
            Err(_) => false,
        };

        if is_contiguous {
            self.state.set_highest_contiguous(block_number, &mut batch)?;
        }

        self.hot_tier.add_block_to_batch(block, block_number, &mut batch)?;
        self.state.set_latest_persisted(block_number, &mut batch)?;
        self.state.initialize_earliest_hot(block_number, &mut batch)?;

        self.hot_tier.commit_batch(batch)?;
        self.archiver.run_archival_cycle()?;

        Ok(())
    }

    fn write_block_batch(&self, blocks: &[Block]) -> Result<()> {
        if blocks.is_empty() {
            return Ok(());
        }

        info!("Writing historical batch of {} blocks directly to cold storage.", blocks.len());

        // Step 1: Write the archive and get the path of the new index file.
        let new_index_path = self.archiver.cold_writer.write_archive(blocks)?;

        // Step 2: Immediately load the new index into the reader's in-memory map.
        if let Err(e) = self.cold_reader.load_index_file(&new_index_path) {
            warn!("CRITICAL: Failed to live-load new index file for historical batch {:?}: {}. A restart may be required to see these blocks.", new_index_path, e);
        } else {
            info!("Cold reader index successfully updated for historical batch.");
        }

        // Step 3: Update the node's true earliest block metadata if this batch is older.
        let batch_earliest = get_block_number(blocks.first().unwrap())?;
        self.state.update_true_earliest_if_less(batch_earliest)?;
        info!("Checked/updated true earliest block number with historical batch.");
        
        // Step 4: Return Ok(()) to satisfy the trait contract.
        Ok(())
    }
}

fn get_block_number(block: &Block) -> Result<u64> {
    if let Some(first_item) = block.items.first() {
        if let Some(block_item::Item::BlockHeader(header)) = &first_item.item {
            return Ok(header.number);
        }
    }
    Err(anyhow!(
        "Block is malformed or first item is not a BlockHeader"
    ))
}
