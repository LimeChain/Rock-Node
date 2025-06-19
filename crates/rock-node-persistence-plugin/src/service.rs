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

/// The main service that orchestrates all persistence operations.
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
    fn get_latest_persisted_block_number(&self) -> i64 {
        self.state.get_latest_persisted().unwrap_or(0) as i64
    }

    fn get_earliest_persisted_block_number(&self) -> i64 {
        self.state.get_earliest_hot().unwrap_or(0) as i64
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

        // **CORRECTION: Implement the contiguity logic**
        // 1. Get the current contiguous watermark.
        let current_contiguous = self.state.get_highest_contiguous()?;

        // 2. Check if the new block is the next one in the sequence.
        if block_number == current_contiguous + 1 {
            // 3. If it is, update the watermark in the same transaction.
            self.state.set_highest_contiguous(block_number, &mut batch)?;
        }
        // Note: A full implementation would buffer out-of-order blocks.
        // For now, we simply don't advance the watermark if a block arrives early.

        self.hot_tier
            .add_block_to_batch(block, block_number, &mut batch)?;
        self.state
            .set_latest_persisted(block_number, &mut batch)?;
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
        // This is a simplified implementation for now. A full implementation
        // would also need to update the contiguous watermark if this batch
        // closes a known gap.
        self.archiver.cold_writer.write_archive(blocks)
    }
}

/// Helper function to safely access the block number from a Block's first item.
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
