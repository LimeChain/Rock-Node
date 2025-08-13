use crate::{
    cold_storage::{reader::ColdReader, writer::ColdWriter},
    hot_tier::HotTier,
    state::StateManager,
};
use anyhow::Result;
use rock_node_core::{config::PersistenceServiceConfig, metrics::MetricsRegistry};
use rocksdb::WriteBatch;
use std::sync::Arc;
use tokio::sync::Notify;
use tracing::{info, trace, warn};

#[derive(Debug)]
pub struct Archiver {
    pub config: Arc<PersistenceServiceConfig>,
    pub hot_tier: Arc<HotTier>,
    pub cold_writer: Arc<ColdWriter>,
    pub state: Arc<StateManager>,
    pub cold_reader: Arc<ColdReader>,
    pub metrics: Arc<MetricsRegistry>,
    pub trigger: Arc<Notify>,
    pub shutdown_notify: Arc<Notify>,
}

impl Archiver {
    pub fn new(
        config: Arc<PersistenceServiceConfig>,
        hot_tier: Arc<HotTier>,
        cold_writer: Arc<ColdWriter>,
        state: Arc<StateManager>,
        cold_reader: Arc<ColdReader>,
        metrics: Arc<MetricsRegistry>,
        shutdown_notify: Arc<Notify>,
    ) -> Self {
        Self {
            config,
            hot_tier,
            cold_writer,
            state,
            cold_reader,
            metrics,
            trigger: Arc::new(Notify::new()),
            shutdown_notify,
        }
    }

    pub fn notify_check(&self) {
        self.trigger.notify_one();
    }

    pub fn run_archival_cycle(&self) -> Result<()> {
        // This loop will continue to process batches as long as the hot tier is oversized
        // and there are contiguous blocks to archive.
        loop {
            let earliest_hot = match self.state.get_earliest_hot()? {
                Some(num) => num,
                None => return Ok(()), // Nothing to archive
            };

            let latest_persisted = self.state.get_latest_persisted()?.unwrap_or(0);
            let hot_tier_total_blocks = latest_persisted.saturating_sub(earliest_hot) + 1;
            self.metrics
                .persistence_hot_tier_block_count
                .set(hot_tier_total_blocks as i64);

            // The condition to archive is when the number of blocks in the hot tier
            // STRICTLY EXCEEDS the target count.
            if hot_tier_total_blocks <= self.config.hot_storage_block_count {
                return Ok(());
            }

            let batch_size = self.config.archive_batch_size;

            if self.hot_tier.is_batch_complete(earliest_hot, batch_size)? {
                trace!(
                    "Found complete batch starting at {}. Archiving.",
                    earliest_hot
                );
                self.archive_batch(earliest_hot, batch_size)?;
                // After a successful archive, we immediately continue the loop
                // to see if another batch can be processed right away.
            } else {
                trace!(
                    "Batch starting at {} is incomplete. Marking as skipped.",
                    earliest_hot
                );
                let mut batch = WriteBatch::default();
                self.state.add_skipped_batch(earliest_hot, &mut batch)?;
                self.hot_tier.commit_batch(batch)?;
                // If the very first batch is incomplete, we can't do any more work in this cycle.
                break;
            }
        }

        Ok(())
    }

    fn archive_batch(&self, start_block: u64, count: u64) -> Result<()> {
        let timer = self
            .metrics
            .persistence_archival_cycle_duration_seconds
            .with_label_values(&[])
            .start_timer();

        let blocks_to_archive = self.hot_tier.read_block_batch(start_block, count)?;
        if blocks_to_archive.is_empty() {
            warn!(
                "Attempted to archive batch from {} but read 0 blocks. Aborting.",
                start_block
            );
            return Ok(());
        }

        let new_index_path = self.cold_writer.write_archive(&blocks_to_archive)?;
        if let Err(e) = self.cold_reader.load_index_file(&new_index_path) {
            warn!(
                "CRITICAL: Failed to live-load new index file {:?}: {}. A restart may be required.",
                new_index_path, e
            );
        }

        let new_earliest_hot = start_block + count;
        let mut batch = WriteBatch::default();
        for i in 0..count {
            self.hot_tier
                .add_delete_to_batch(start_block + i, &mut batch)?;
        }
        self.state.set_earliest_hot(new_earliest_hot, &mut batch)?;
        self.state.remove_skipped_batch(start_block, &mut batch)?;

        self.hot_tier.commit_batch(batch)?;

        info!(
            "Archival cycle complete. New earliest hot block is #{}.",
            new_earliest_hot
        );
        timer.observe_duration();
        self.metrics.persistence_archival_cycles_total.inc();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;
    use rock_node_core::database::DatabaseManager;
    use tempfile::TempDir;

    fn make_block(num: u64) -> rock_node_protobufs::com::hedera::hapi::block::stream::Block {
        rock_node_protobufs::com::hedera::hapi::block::stream::Block {
            items: vec![rock_node_protobufs::com::hedera::hapi::block::stream::BlockItem {
                item: Some(
                    rock_node_protobufs::com::hedera::hapi::block::stream::block_item::Item::BlockHeader(
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

    #[test]
    fn archival_cycle_moves_complete_batch_and_updates_state() {
        let tmp_dir = TempDir::new().unwrap();
        let db = DatabaseManager::new(tmp_dir.path().to_str().unwrap()).unwrap().db_handle();
        let state = Arc::new(StateManager::new(db.clone()));
        let hot = Arc::new(HotTier::new(db.clone()));
        let metrics = Arc::new(MetricsRegistry::new().unwrap());
        let config = Arc::new(PersistenceServiceConfig {
            enabled: true,
            cold_storage_path: tmp_dir.path().to_str().unwrap().to_string(),
            hot_storage_block_count: 2,
            archive_batch_size: 3,
        });
        let cold_writer = Arc::new(ColdWriter::new(config.clone()));
        let cold_reader = Arc::new(ColdReader::new(config.clone(), metrics.clone()));
        let archiver = Archiver::new(
            config,
            hot.clone(),
            cold_writer,
            state.clone(),
            cold_reader.clone(),
            metrics,
            Arc::new(Notify::new()),
        );

        // Seed state: earliest_hot=100, latest_persisted=104 (5 blocks)
        let mut batch = WriteBatch::default();
        state.set_earliest_hot(100, &mut batch).unwrap();
        state.set_latest_persisted(104, &mut batch).unwrap();
        // Add five blocks to hot tier
        for n in 100..=104 {
            let b = make_block(n);
            hot.add_block_to_batch(&b, n, &mut batch).unwrap();
        }
        hot.commit_batch(batch).unwrap();

        // Hot tier has 5 blocks, limit is 2 => archive should proceed.
        archiver.run_archival_cycle().unwrap();

        // After archiving 3-block batch starting at 100, earliest_hot should be 103
        assert_eq!(state.get_earliest_hot().unwrap(), Some(103));
        // And those first three should be gone from hot tier
        assert!(hot.read_block(100).unwrap().is_none());
        assert!(hot.read_block(101).unwrap().is_none());
        assert!(hot.read_block(102).unwrap().is_none());
        // Remaining still present
        assert!(hot.read_block(103).unwrap().is_some());
        assert!(hot.read_block(104).unwrap().is_some());

        // Cold reader should be able to read an archived block
        let bytes = cold_reader.read_block(101).unwrap().unwrap();
        let decoded = rock_node_protobufs::com::hedera::hapi::block::stream::Block::decode(bytes.as_slice()).unwrap();
        match decoded.items.first().unwrap().item.as_ref().unwrap() {
            rock_node_protobufs::com::hedera::hapi::block::stream::block_item::Item::BlockHeader(h) => assert_eq!(h.number, 101),
            _ => panic!("unexpected item"),
        }
    }
}
