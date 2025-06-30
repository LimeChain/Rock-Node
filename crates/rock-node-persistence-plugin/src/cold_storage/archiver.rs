use crate::{cold_storage::writer::ColdWriter, hot_tier::HotTier, state::StateManager};
use anyhow::Result;
use rock_node_core::config::PersistenceServiceConfig;
use rocksdb::WriteBatch;
use std::sync::Arc;
use tracing::info;

#[derive(Debug, Clone)]
pub struct Archiver {
    pub config: Arc<PersistenceServiceConfig>,
    pub hot_tier: Arc<HotTier>,
    pub cold_writer: Arc<ColdWriter>,
    pub state: Arc<StateManager>,
}

impl Archiver {
    pub fn new(
        config: Arc<PersistenceServiceConfig>,
        hot_tier: Arc<HotTier>,
        cold_writer: Arc<ColdWriter>,
        state: Arc<StateManager>,
    ) -> Self {
        Self {
            config,
            hot_tier,
            cold_writer,
            state,
        }
    }

    // CHANGED: Logic now handles Option<u64> instead of i64.
    pub fn run_archival_cycle(&self) -> Result<()> {
        let latest_hot_opt = self.state.get_latest_persisted()?;
        let earliest_hot_opt = self.state.get_earliest_hot()?;

        // Use a `let...else` block for cleaner guard clauses.
        let (latest_hot_u64, earliest_hot_u64) =
            if let (Some(latest), Some(earliest)) = (latest_hot_opt, earliest_hot_opt) {
                (latest, earliest)
            } else {
                return Ok(()); // Nothing to archive if either boundary is unknown.
            };

        if latest_hot_u64 <= earliest_hot_u64 {
            return Ok(()); // Or if hot tier is empty/has one block.
        }

        let current_block_count = latest_hot_u64.saturating_sub(earliest_hot_u64) + 1;

        if current_block_count
            < self.config.hot_storage_block_count + self.config.archive_batch_size
        {
            return Ok(()); // Not enough blocks to form a full archive batch yet.
        }

        info!(
            "Hot tier count ({}) exceeds trigger threshold ({}), starting archival...",
            current_block_count,
            self.config.hot_storage_block_count + self.config.archive_batch_size
        );

        let num_to_archive = self.config.archive_batch_size;
        let end_block_to_archive = earliest_hot_u64 + num_to_archive;

        let blocks_to_archive = self
            .hot_tier
            .read_block_batch(earliest_hot_u64, num_to_archive)?;
        info!(
            "Read {} blocks from hot tier for archival ({} to {}).",
            blocks_to_archive.len(),
            earliest_hot_u64,
            end_block_to_archive - 1
        );

        self.cold_writer.write_archive(&blocks_to_archive)?;
        info!("Successfully wrote blocks to cold storage archive.");

        let mut batch = WriteBatch::default();
        for i in 0..num_to_archive {
            self.hot_tier
                .add_delete_to_batch(earliest_hot_u64 + i, &mut batch)?;
        }
        self.state
            .set_earliest_hot(end_block_to_archive, &mut batch)?;

        self.hot_tier.commit_batch(batch)?;
        info!(
            "Archival cycle complete. New earliest hot block is #{}.",
            end_block_to_archive
        );

        Ok(())
    }
}
