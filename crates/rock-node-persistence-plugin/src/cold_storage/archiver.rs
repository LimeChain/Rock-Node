use crate::{
    cold_storage::writer::ColdWriter,
    hot_tier::HotTier,
    state::StateManager,
};
use anyhow::Result;
use rock_node_core::config::PersistenceServiceConfig;
use std::sync::Arc;
use rocksdb::WriteBatch;
use tracing::info;

/// Responsible for moving blocks from the hot tier to the cold tier.
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
        Self { config, hot_tier, cold_writer, state }
    }

    /// Checks if archival is needed and runs the process if so.
    pub fn run_archival_cycle(&self) -> Result<()> {
        let latest_hot = self.state.get_latest_persisted()?;
        let earliest_hot = self.state.get_earliest_hot()?;

        if earliest_hot == 0 || latest_hot <= earliest_hot {
            return Ok(()); // Nothing to archive
        }

        let current_block_count = latest_hot.saturating_sub(earliest_hot) + 1;
        if current_block_count <= self.config.hot_storage_block_count {
            return Ok(()); // Below threshold
        }
        
        info!("Hot tier count ({}) exceeds limit ({}), starting archival...", current_block_count, self.config.hot_storage_block_count);

        let num_to_archive = std::cmp::min(
            current_block_count - self.config.hot_storage_block_count,
            self.config.archive_batch_size,
        );

        if num_to_archive == 0 {
            return Ok(());
        }

        let end_block_to_archive = earliest_hot + num_to_archive;

        let blocks_to_archive = self.hot_tier.read_block_batch(earliest_hot, num_to_archive)?;
        info!("Read {} blocks from hot tier for archival ({} to {}).", blocks_to_archive.len(), earliest_hot, end_block_to_archive - 1);

        self.cold_writer.write_archive(&blocks_to_archive)?;
        info!("Successfully wrote blocks to cold storage archive.");

        let mut batch = WriteBatch::default();
        for i in 0..num_to_archive {
            self.hot_tier.add_delete_to_batch(earliest_hot + i, &mut batch)?;
        }
        self.state.set_earliest_hot(end_block_to_archive, &mut batch)?;
        
        self.hot_tier.commit_batch(batch)?;
        info!("Archival cycle complete. New earliest hot block is #{}.", end_block_to_archive);

        Ok(())
    }
}
