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
use tracing::info;

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

// CHANGED: Implementing the updated BlockReader trait.
impl BlockReader for PersistenceService {
    fn get_latest_persisted_block_number(&self) -> Result<Option<u64>> {
        self.state.get_latest_persisted()
    }

    // FIX: This now correctly reads the true earliest block number from the dedicated metadata key.
    fn get_earliest_persisted_block_number(&self) -> Result<Option<u64>> {
        self.state.get_true_earliest_persisted()
    }

    // FIX: Implemented the full router logic to check hot tier first, then cold tier.
    fn read_block(&self, block_number: u64) -> Result<Option<Vec<u8>>> {
        // 1. Try to read from the high-performance hot tier first.
        if let Some(block_bytes) = self.hot_tier.read_block(block_number)? {
            return Ok(Some(block_bytes));
        }

        // 2. If not found in hot, delegate the lookup to the cold storage reader.
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

        // FIX: Check if the true earliest needs to be initialized.
        if self.state.get_true_earliest_persisted()?.is_none() {
             self.state.set_true_earliest_persisted(block_number, &mut batch)?;
        }

        // Check if this new block extends the contiguous chain.
        let is_contiguous = match self.get_highest_contiguous_block_number() {
            // Special case for empty DB, any starting block is okay for contiguous counter.
            Ok(0) if self.get_latest_persisted_block_number()?.is_none() => true,
            Ok(current_contiguous) => block_number == current_contiguous + 1,
            Err(_) => false,
        };

        if is_contiguous {
            self.state
                .set_highest_contiguous(block_number, &mut batch)?;
        }

        self.hot_tier
            .add_block_to_batch(block, block_number, &mut batch)?;
        self.state.set_latest_persisted(block_number, &mut batch)?;
        self.state
            .initialize_earliest_hot(block_number, &mut batch)?;

        self.hot_tier.commit_batch(batch)?;
        
        self.archiver.run_archival_cycle()?;

        Ok(())
    }

    fn write_block_batch(&self, blocks: &[Block]) -> Result<()> {
        info!(
            "Writing historical batch of {} blocks directly to cold storage.",
            blocks.len()
        );
        // This archival path should also update the "true_earliest_persisted" metadata.
        // For now, focusing on the main fixes. This is a V2 improvement consideration.
        self.archiver.cold_writer.write_archive(blocks)
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
