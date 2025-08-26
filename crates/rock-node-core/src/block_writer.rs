use anyhow::Result;
use async_trait::async_trait;
use rock_node_protobufs::com::hedera::hapi::block::stream::Block;
use std::fmt::Debug;
use std::sync::Arc;

/// The write-path interface for the Persistence Service.
/// This trait defines the contract for any plugin that needs to submit blocks for storage.
#[async_trait]
pub trait BlockWriter: Debug + Send + Sync + 'static {
    /// Stores a single block. Typically used by the live ingestion pipeline.
    async fn write_block(&self, block: &Block) -> Result<()>;

    /// Stores a batch of blocks. Optimized for historical backfilling.
    /// The implementation should be smart enough to route blocks to the
    /// correct tier (hot or cold) based on their block number.
    async fn write_block_batch(&self, blocks: &[Block]) -> Result<()>;
}

/// A concrete, shareable handle for the BlockWriter service.
#[derive(Clone, Debug)]
pub struct BlockWriterProvider {
    writer: Arc<dyn BlockWriter>,
}

impl BlockWriterProvider {
    pub fn new(writer: Arc<dyn BlockWriter>) -> Self {
        Self { writer }
    }

    pub fn get_writer(&self) -> Arc<dyn BlockWriter> {
        self.writer.clone()
    }
}
