use crate::{
    cold_storage::{archiver::Archiver, reader::ColdReader},
    hot_tier::HotTier,
    state::StateManager,
};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
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

#[async_trait]
impl BlockWriter for PersistenceService {
    async fn write_block(&self, block: &Block) -> Result<()> {
        let timer = self
            .metrics
            .persistence_write_duration_seconds
            .with_label_values(&["live"])
            .start_timer();
        let block_number = get_block_number(block)?;

        // Clone Arcs to move them into the blocking task
        let state = self.state.clone();
        let hot_tier = self.hot_tier.clone();
        let archiver = self.archiver.clone();
        let service_clone = self.clone();
        let block = block.clone();

        // Move the synchronous work to a blocking thread
        tokio::task::spawn_blocking(move || -> Result<()> { // <-- FIX: Add explicit return type
            let mut batch = WriteBatch::default();

            let was_gap_fill = state.find_containing_gap(block_number)?.is_some();

            hot_tier.add_block_to_batch(&block, block_number, &mut batch)?;

            let latest_persisted = state
                .get_latest_persisted()?
                .unwrap_or(service_clone.start_block_number.saturating_sub(1));
            let new_latest_persisted = std::cmp::max(latest_persisted, block_number);

            if new_latest_persisted > latest_persisted {
                state.set_latest_persisted(new_latest_persisted, &mut batch)?;
            }

            if state.get_true_earliest_persisted()?.is_none() {
                state.set_true_earliest_persisted(block_number, &mut batch)?;
            }

            let highest_contiguous = state.get_highest_contiguous()?;
            if block_number > highest_contiguous + 1 {
                state.add_gap_range(highest_contiguous + 1, block_number - 1, &mut batch)?;
            } else {
                state.fill_gap_block(block_number, &mut batch)?;
            }

            service_clone.advance_highest_contiguous(new_latest_persisted, &mut batch)?;
            state.initialize_earliest_hot(block_number, &mut batch)?;

            hot_tier.commit_batch(batch)?;

            // Trigger the archiver only if a gap was filled that might complete a batch.
            if was_gap_fill {
                let batch_size = archiver.config.archive_batch_size;
                let batch_start = (block_number / batch_size) * batch_size;
                if state.is_batch_skipped(batch_start)? {
                    trace!(
                        "Gap fill for block #{} may have completed a skipped batch. Triggering archiver.",
                        block_number
                    );
                    archiver.notify_check();
                }
            }
            Ok(())
        })
        .await??; // The first '?' handles JoinError, the second handles the inner Result

        timer.observe_duration();
        self.metrics
            .persistence_writes_total
            .with_label_values(&["live"])
            .inc();
        Ok(())
    }

    async fn write_block_batch(&self, blocks: &[Block]) -> Result<()> {
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

        // This is also blocking I/O and should be handled similarly
        let archiver = self.archiver.clone();
        let cold_reader = self.cold_reader.clone();
        let state = self.state.clone();
        let blocks = blocks.to_vec(); // Clone to move into the task

        tokio::task::spawn_blocking(move || -> Result<()> { // <-- FIX: Add explicit return type
            let new_index_path = archiver.cold_writer.write_archive(&blocks)?;
            if let Err(e) = cold_reader.load_index_file(&new_index_path) {
                warn!("CRITICAL: Failed to live-load new index file for historical batch {:?}: {}. A restart may be required to see these blocks.", new_index_path, e);
            } else {
                trace!("Cold reader index successfully updated for historical batch.");
            }
            let batch_earliest = get_block_number(
                blocks
                    .first()
                    .ok_or_else(|| anyhow!("Archive batch is empty"))?,
            )?;
            state.update_true_earliest_if_less(batch_earliest)?;
            trace!("Checked/updated true earliest block number with historical batch.");
            Ok(())
        }).await??;

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

#[cfg(test)]
mod tests {
    use super::*;
    use rock_node_core::database::DatabaseManager;
    use tempfile::TempDir;

    fn make_block(num: u64) -> Block {
        Block {
            items: vec![rock_node_protobufs::com::hedera::hapi::block::stream::BlockItem {
                item: Some(
                    block_item::Item::BlockHeader(
                        rock_node_protobufs::com::hedera::hapi::block::stream::output::BlockHeader {
                            hapi_proto_version: None,
                            software_version: None,
                            number: num,
                            block_timestamp: None,
                            hash_algorithm: 0,
                        },
                    ),
                ),
            }],
        }
    }

    fn make_service(tmp: &TempDir, start_block: u64) -> PersistenceService {
        let db = DatabaseManager::new(tmp.path().to_str().unwrap())
            .unwrap()
            .db_handle();
        let metrics = Arc::new(MetricsRegistry::new().unwrap());
        let state = Arc::new(StateManager::new(db.clone()));
        state
            .initialize_highest_contiguous(start_block.saturating_sub(1))
            .unwrap();
        let hot = Arc::new(HotTier::new(db.clone()));
        let config = Arc::new(rock_node_core::config::PersistenceServiceConfig {
            enabled: true,
            cold_storage_path: tmp.path().to_str().unwrap().to_string(),
            hot_storage_block_count: 10,
            archive_batch_size: 5,
        });
        let cold_writer = Arc::new(crate::cold_storage::writer::ColdWriter::new(config.clone()));
        let cold_reader = Arc::new(crate::cold_storage::reader::ColdReader::new(
            config.clone(),
            metrics.clone(),
        ));
        let archiver = Arc::new(crate::cold_storage::archiver::Archiver::new(
            config,
            hot.clone(),
            cold_writer,
            state.clone(),
            cold_reader.clone(),
            metrics.clone(),
            Arc::new(tokio::sync::Notify::new()),
        ));
        PersistenceService::new(hot, cold_reader, archiver, state, metrics, start_block)
    }

    #[tokio::test]
    async fn write_and_read_blocks_updates_state_and_gaps() {
        let tmp = TempDir::new().unwrap();
        let service = make_service(&tmp, 100);

        // write 100 and 102 to create a gap at 101
        service.write_block(&make_block(100)).await.unwrap();
        service.write_block(&make_block(102)).await.unwrap();

        assert_eq!(
            service.get_latest_persisted_block_number().unwrap(),
            Some(102)
        );
        assert_eq!(service.get_highest_contiguous_block_number().unwrap(), 100);
        assert!(service.state.find_containing_gap(101).unwrap().is_some());

        // Fill the gap with 101 and ensure highest_contiguous advances to 102
        service.write_block(&make_block(101)).await.unwrap();
        assert_eq!(service.get_highest_contiguous_block_number().unwrap(), 102);

        // Read from hot tier
        assert!(service.read_block(100).unwrap().is_some());
        assert!(service.read_block(101).unwrap().is_some());
        assert!(service.read_block(102).unwrap().is_some());
    }

    #[tokio::test]
    async fn write_block_batch_updates_true_earliest() {
        let tmp = TempDir::new().unwrap();
        let service = make_service(&tmp, 50);
        let blocks: Vec<Block> = (40..45).map(make_block).collect();
        service.write_block_batch(&blocks).await.unwrap();
        assert_eq!(
            service.get_earliest_persisted_block_number().unwrap(),
            Some(40)
        );
    }
}
