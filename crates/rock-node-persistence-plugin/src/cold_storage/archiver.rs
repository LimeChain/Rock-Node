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
use tracing::{debug, trace, warn};

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

    /// Wakes up the archiver task. This is cheap and can be called frequently.
    pub fn notify_check(&self) {
        self.trigger.notify_one();
    }

    pub fn run_archival_cycle(&self) -> Result<()> {
        let latest_hot_opt = self.state.get_latest_persisted()?;
        let earliest_hot_opt = self.state.get_earliest_hot()?;

        let (latest_hot_u64, earliest_hot_u64) =
            if let (Some(latest), Some(earliest)) = (latest_hot_opt, earliest_hot_opt) {
                (latest, earliest)
            } else {
                // Not enough data to archive, which is normal.
                return Ok(());
            };

        if latest_hot_u64 <= earliest_hot_u64 {
            return Ok(());
        }

        let current_block_count = latest_hot_u64.saturating_sub(earliest_hot_u64) + 1;
        self.metrics
            .persistence_hot_tier_block_count
            .set(current_block_count as i64);

        if current_block_count
            < self.config.hot_storage_block_count + self.config.archive_batch_size
        {
            // Not enough blocks to trigger an archival run.
            return Ok(());
        }

        // --- Gap-Aware Check ---
        // TODO: We still need to archive batches ahead that are contiguous, but not yet persisted.
        let highest_contiguous = self.state.get_highest_contiguous()?;
        let end_of_batch_to_archive = earliest_hot_u64 + self.config.archive_batch_size - 1;

        if end_of_batch_to_archive > highest_contiguous {
            debug!(
                "Archival paused. Batch end #{} is beyond highest contiguous block #{}. Waiting for backfill.",
                end_of_batch_to_archive, highest_contiguous
            );
            return Ok(());
        }
        // --- End Gap-Aware Check ---

        let timer = self
            .metrics
            .persistence_archival_cycle_duration_seconds
            .with_label_values(&[])
            .start_timer();

        trace!(
            "Hot tier count ({}) exceeds trigger, starting archival...",
            current_block_count
        );

        let num_to_archive = self.config.archive_batch_size;
        let blocks_to_archive = self
            .hot_tier
            .read_block_batch(earliest_hot_u64, num_to_archive)?;
        trace!(
            "Read {} blocks from hot tier for archival.",
            blocks_to_archive.len()
        );

        let new_index_path = self.cold_writer.write_archive(&blocks_to_archive)?;
        trace!(
            "Successfully wrote blocks to cold archive: {:?}",
            new_index_path.file_name().unwrap_or_default()
        );

        if let Err(e) = self.cold_reader.load_index_file(&new_index_path) {
            warn!("CRITICAL: Failed to live-load new index file {:?}: {}. A restart may be required to see these blocks.", new_index_path, e);
        } else {
            trace!("Cold reader index successfully updated in real-time.");
        }

        let end_block_to_archive = earliest_hot_u64 + num_to_archive;
        let mut batch = WriteBatch::default();
        for i in 0..num_to_archive {
            self.hot_tier
                .add_delete_to_batch(earliest_hot_u64 + i, &mut batch)?;
        }
        self.state
            .set_earliest_hot(end_block_to_archive, &mut batch)?;

        self.hot_tier.commit_batch(batch)?;
        trace!(
            "Archival cycle complete. New earliest hot block is #{}.",
            end_block_to_archive
        );

        timer.observe_duration();
        self.metrics.persistence_archival_cycles_total.inc();

        Ok(())
    }
}
