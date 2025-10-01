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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::RwLock;

    /// A simple mock implementation of BlockWriter for testing
    #[derive(Debug)]
    struct MockBlockWriter {
        blocks: RwLock<Vec<Block>>,
        should_fail: RwLock<bool>,
    }

    impl MockBlockWriter {
        fn new() -> Self {
            Self {
                blocks: RwLock::new(Vec::new()),
                should_fail: RwLock::new(false),
            }
        }

        fn set_should_fail(&self, should_fail: bool) {
            *self.should_fail.write().unwrap() = should_fail;
        }

        fn get_written_blocks(&self) -> Vec<Block> {
            self.blocks.read().unwrap().clone()
        }

        fn clear(&self) {
            self.blocks.write().unwrap().clear();
        }
    }

    #[async_trait]
    impl BlockWriter for MockBlockWriter {
        async fn write_block(&self, block: &Block) -> Result<()> {
            if *self.should_fail.read().unwrap() {
                return Err(anyhow::anyhow!("Mock write failure"));
            }
            self.blocks.write().unwrap().push(block.clone());
            Ok(())
        }

        async fn write_block_batch(&self, blocks: &[Block]) -> Result<()> {
            if *self.should_fail.read().unwrap() {
                return Err(anyhow::anyhow!("Mock batch write failure"));
            }
            self.blocks.write().unwrap().extend_from_slice(blocks);
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_block_writer_provider_creation() {
        let mock_writer = Arc::new(MockBlockWriter::new());
        let provider = BlockWriterProvider::new(mock_writer);
        assert!(format!("{:?}", provider).contains("BlockWriterProvider"));
    }

    #[tokio::test]
    async fn test_block_writer_provider_get_writer() {
        let mock_writer = Arc::new(MockBlockWriter::new());
        let provider = BlockWriterProvider::new(mock_writer);

        let writer = provider.get_writer();
        let block = Block::default();
        assert!(writer.write_block(&block).await.is_ok());
    }

    #[tokio::test]
    async fn test_block_writer_provider_clone() {
        let mock_writer = Arc::new(MockBlockWriter::new());
        let provider = BlockWriterProvider::new(mock_writer);
        let provider_clone = provider.clone();

        let writer1 = provider.get_writer();
        let writer2 = provider_clone.get_writer();

        let block = Block::default();
        assert!(writer1.write_block(&block).await.is_ok());
        assert!(writer2.write_block(&block).await.is_ok());
    }

    #[tokio::test]
    async fn test_mock_block_writer_write_single_block() {
        let writer = MockBlockWriter::new();
        let block = Block::default();

        assert!(writer.write_block(&block).await.is_ok());
        assert_eq!(writer.get_written_blocks().len(), 1);
    }

    #[tokio::test]
    async fn test_mock_block_writer_write_multiple_blocks() {
        let writer = MockBlockWriter::new();

        for _ in 0..5 {
            let block = Block::default();
            assert!(writer.write_block(&block).await.is_ok());
        }

        assert_eq!(writer.get_written_blocks().len(), 5);
    }

    #[tokio::test]
    async fn test_mock_block_writer_write_batch() {
        let writer = MockBlockWriter::new();
        let blocks = vec![Block::default(), Block::default(), Block::default()];

        assert!(writer.write_block_batch(&blocks).await.is_ok());
        assert_eq!(writer.get_written_blocks().len(), 3);
    }

    #[tokio::test]
    async fn test_mock_block_writer_write_empty_batch() {
        let writer = MockBlockWriter::new();
        let blocks: Vec<Block> = vec![];

        assert!(writer.write_block_batch(&blocks).await.is_ok());
        assert_eq!(writer.get_written_blocks().len(), 0);
    }

    #[tokio::test]
    async fn test_mock_block_writer_write_large_batch() {
        let writer = MockBlockWriter::new();
        let blocks = vec![Block::default(); 100];

        assert!(writer.write_block_batch(&blocks).await.is_ok());
        assert_eq!(writer.get_written_blocks().len(), 100);
    }

    #[tokio::test]
    async fn test_mock_block_writer_mixed_operations() {
        let writer = MockBlockWriter::new();

        // Write single block
        writer.write_block(&Block::default()).await.unwrap();

        // Write batch
        writer
            .write_block_batch(&[Block::default(), Block::default()])
            .await
            .unwrap();

        // Write another single block
        writer.write_block(&Block::default()).await.unwrap();

        assert_eq!(writer.get_written_blocks().len(), 4);
    }

    #[tokio::test]
    async fn test_mock_block_writer_clear() {
        let writer = MockBlockWriter::new();
        writer.write_block(&Block::default()).await.unwrap();
        assert_eq!(writer.get_written_blocks().len(), 1);

        writer.clear();
        assert_eq!(writer.get_written_blocks().len(), 0);
    }

    #[tokio::test]
    async fn test_mock_block_writer_failure() {
        let writer = MockBlockWriter::new();
        writer.set_should_fail(true);

        let result = writer.write_block(&Block::default()).await;
        assert!(result.is_err());
        assert_eq!(writer.get_written_blocks().len(), 0);
    }

    #[tokio::test]
    async fn test_mock_block_writer_batch_failure() {
        let writer = MockBlockWriter::new();
        writer.set_should_fail(true);

        let blocks = vec![Block::default(), Block::default()];
        let result = writer.write_block_batch(&blocks).await;
        assert!(result.is_err());
        assert_eq!(writer.get_written_blocks().len(), 0);
    }

    #[tokio::test]
    async fn test_mock_block_writer_failure_recovery() {
        let writer = MockBlockWriter::new();

        // First write succeeds
        writer.write_block(&Block::default()).await.unwrap();

        // Enable failures
        writer.set_should_fail(true);
        assert!(writer.write_block(&Block::default()).await.is_err());

        // Disable failures and try again
        writer.set_should_fail(false);
        writer.write_block(&Block::default()).await.unwrap();

        assert_eq!(writer.get_written_blocks().len(), 2);
    }

    #[tokio::test]
    async fn test_block_writer_trait_send_sync() {
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}
        assert_send::<Arc<dyn BlockWriter>>();
        assert_sync::<Arc<dyn BlockWriter>>();
    }

    #[tokio::test]
    async fn test_provider_shares_underlying_writer() {
        let mock_writer = Arc::new(MockBlockWriter::new());

        let provider1 = BlockWriterProvider::new(mock_writer.clone());
        let provider2 = BlockWriterProvider::new(mock_writer.clone());

        let writer1 = provider1.get_writer();
        let writer2 = provider2.get_writer();

        writer1.write_block(&Block::default()).await.unwrap();
        writer2.write_block(&Block::default()).await.unwrap();

        // Both should write to the same underlying storage
        assert_eq!(mock_writer.get_written_blocks().len(), 2);
    }

    #[tokio::test]
    async fn test_provider_concurrent_writes() {
        let mock_writer = Arc::new(MockBlockWriter::new());
        let provider = BlockWriterProvider::new(mock_writer.clone());

        let mut handles = vec![];
        for _ in 0..10 {
            let writer = provider.get_writer();
            let handle = tokio::spawn(async move { writer.write_block(&Block::default()).await });
            handles.push(handle);
        }

        for handle in handles {
            assert!(handle.await.unwrap().is_ok());
        }

        assert_eq!(mock_writer.get_written_blocks().len(), 10);
    }

    #[tokio::test]
    async fn test_provider_debug_format() {
        let mock_writer = Arc::new(MockBlockWriter::new());
        let provider = BlockWriterProvider::new(mock_writer);
        let debug_str = format!("{:?}", provider);
        assert!(debug_str.contains("BlockWriterProvider"));
    }
}
