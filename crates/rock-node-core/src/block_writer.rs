use anyhow::Result;
use rock_node_protobufs::com::hedera::hapi::block::stream::Block;
use std::fmt::Debug;

/// The write-path interface for the Persistence Service.
/// This trait defines the contract for any plugin that needs to submit blocks for storage.
pub trait BlockWriter: Debug + Send + Sync + 'static {
    /// Stores a single block. Typically used by the live ingestion pipeline.
    fn write_block(&self, block: &Block) -> Result<()>;

    /// Stores a batch of blocks. Optimized for historical backfilling.
    /// The implementation should be smart enough to route blocks to the
    /// correct tier (hot or cold) based on their block number.
    fn write_block_batch(&self, blocks: &[Block]) -> Result<()>;
}
