use crate::{
    cold_storage::{archiver::Archiver, reader::ColdReader},
    hot_tier::HotTier,
    state::StateManager,
};
use anyhow::{anyhow, Result};
use rock_node_core::{
    block_reader::BlockReader, block_writer::BlockWriter, metrics::MetricsRegistry,
};
use rock_node_protobufs::com::hedera::hapi::block::stream::{block_item, Block};
use rocksdb::WriteBatch;
use std::sync::Arc;
use tracing::{trace, warn};

#[derive(Debug, Clone)]
pub struct PersistenceService {
    hot_tier: Arc<HotTier>,
    cold_reader: Arc<ColdReader>,
    archiver: Arc<Archiver>,
    state: Arc<StateManager>,
    metrics: Arc<MetricsRegistry>,
    start_block_number: u64,
}

impl PersistenceService {
    pub fn new(
        hot_tier: Arc<HotTier>,
        cold_reader: Arc<ColdReader>,
        archiver: Arc<Archiver>,
        state: Arc<StateManager>,
        metrics: Arc<MetricsRegistry>,
        start_block_number: u64,
    ) -> Self {
        Self {
            hot_tier,
            cold_reader,
            archiver,
            state,
            metrics,
            start_block_number,
        }
    }

    /// Advances the highest contiguous block number as far as possible,
    /// bounded by the latest known persisted block. This prevents infinite loops.
    fn advance_highest_contiguous(
        &self,
        latest_known_persisted: u64,
        batch: &mut WriteBatch,
    ) -> Result<()> {
        let mut current_highest = self.state.get_highest_contiguous()?;
        let mut next_to_check = current_highest + 1;

        // The loop is bounded by latest_known_persisted, which is critical to prevent hangs.
        while next_to_check <= latest_known_persisted {
            // A block is considered "present" if it is NOT in a known gap.
            if self.state.find_containing_gap(next_to_check)?.is_some() {
                // We've hit a known gap, so we cannot advance further.
                break;
            }
            // If it's not a gap, we can advance the contiguous counter.
            current_highest = next_to_check;
            next_to_check += 1;
        }
        self.state.set_highest_contiguous(current_highest, batch)?;
        Ok(())
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
        let hot_timer = self
            .metrics
            .persistence_read_duration_seconds
            .with_label_values(&["hot"])
            .start_timer();
        if let Some(block_bytes) = self.hot_tier.read_block(block_number)? {
            hot_timer.observe_duration();
            self.metrics
                .persistence_reads_total
                .with_label_values(&["hot"])
                .inc();
            return Ok(Some(block_bytes));
        }
        hot_timer.observe_duration();

        let cold_timer = self
            .metrics
            .persistence_read_duration_seconds
            .with_label_values(&["cold"])
            .start_timer();
        if let Some(block_bytes) = self.cold_reader.read_block(block_number)? {
            cold_timer.observe_duration();
            self.metrics
                .persistence_reads_total
                .with_label_values(&["cold"])
                .inc();
            return Ok(Some(block_bytes));
        }
        cold_timer.observe_duration();

        self.metrics
            .persistence_reads_total
            .with_label_values(&["not_found"])
            .inc();
        Ok(None)
    }

    fn get_highest_contiguous_block_number(&self) -> Result<u64> {
        self.state.get_highest_contiguous()
    }
}

impl BlockWriter for PersistenceService {
    fn write_block(&self, block: &Block) -> Result<()> {
        let timer = self
            .metrics
            .persistence_write_duration_seconds
            .with_label_values(&["live"])
            .start_timer();
        let block_number = get_block_number(block)?;
        let mut batch = WriteBatch::default();

        let was_gap_fill = self.state.find_containing_gap(block_number)?.is_some();

        self.hot_tier
            .add_block_to_batch(block, block_number, &mut batch)?;

        let latest_persisted = self
            .get_latest_persisted_block_number()?
            .unwrap_or(self.start_block_number.saturating_sub(1));
        let new_latest_persisted = std::cmp::max(latest_persisted, block_number);

        if new_latest_persisted > latest_persisted {
            self.state
                .set_latest_persisted(new_latest_persisted, &mut batch)?;
        }

        if self.state.get_true_earliest_persisted()?.is_none() {
            self.state
                .set_true_earliest_persisted(block_number, &mut batch)?;
        }

        let highest_contiguous = self.state.get_highest_contiguous()?;
        if block_number > highest_contiguous + 1 {
            self.state
                .add_gap_range(highest_contiguous + 1, block_number - 1, &mut batch)?;
        } else {
            self.state.fill_gap_block(block_number, &mut batch)?;
        }

        // Pass the correct upper bound to prevent the infinite loop.
        self.advance_highest_contiguous(new_latest_persisted, &mut batch)?;
        self.state
            .initialize_earliest_hot(block_number, &mut batch)?;

        self.hot_tier.commit_batch(batch)?;

        // Trigger the archiver only if a gap was filled that might complete a batch.
        if was_gap_fill {
            let batch_size = self.archiver.config.archive_batch_size;
            let batch_start = (block_number / batch_size) * batch_size;
            if self.state.is_batch_skipped(batch_start)? {
                trace!(
                    "Gap fill for block #{} may have completed a skipped batch. Triggering archiver.",
                    block_number
                );
                self.archiver.notify_check();
            }
        }

        timer.observe_duration();
        self.metrics
            .persistence_writes_total
            .with_label_values(&["live"])
            .inc();
        Ok(())
    }

    fn write_block_batch(&self, blocks: &[Block]) -> Result<()> {
        if blocks.is_empty() {
            return Ok(());
        }
        let timer = self
            .metrics
            .persistence_write_duration_seconds
            .with_label_values(&["batch"])
            .start_timer();
        trace!(
            "Writing historical batch of {} blocks directly to cold storage.",
            blocks.len()
        );
        let new_index_path = self.archiver.cold_writer.write_archive(blocks)?;
        if let Err(e) = self.cold_reader.load_index_file(&new_index_path) {
            warn!("CRITICAL: Failed to live-load new index file for historical batch {:?}: {}. A restart may be required to see these blocks.", new_index_path, e);
        } else {
            trace!("Cold reader index successfully updated for historical batch.");
        }
        let batch_earliest = get_block_number(
            blocks
                .first()
                .ok_or_else(|| anyhow!("Archive batch is empty"))?,
        )?;
        self.state.update_true_earliest_if_less(batch_earliest)?;
        trace!("Checked/updated true earliest block number with historical batch.");
        timer.observe_duration();
        self.metrics
            .persistence_writes_total
            .with_label_values(&["batch"])
            .inc();
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
