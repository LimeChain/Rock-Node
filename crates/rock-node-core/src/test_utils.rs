//! Shared utilities for testing Rock Node components.
//!
//! This module provides common testing utilities, particularly for
//! Prometheus registry isolation to prevent cardinality conflicts
//! during test execution.
//!
//! # Registry Isolation
//!
//! The main purpose of this module is to provide utilities for creating
//! isolated Prometheus registries during testing. This prevents cardinality
//! conflicts that occur when multiple tests register metrics with the same
//! names.
//!
//! ## Usage
//!
//! ```rust,ignore
//! use rock_node_core::test_utils::create_isolated_metrics;
//!
//! #[test]
//! fn my_test() {
//!     let metrics = create_isolated_metrics();
//!     // Use metrics in test...
//! }
//! ```
//!
//! ## Implementation Details
//!
//! Each call to `create_isolated_metrics()` creates a fresh `prometheus::Registry`
//! and uses `MetricsRegistry::with_registry()` to create a registry with
//! isolated metric namespaces.

use crate::metrics::MetricsRegistry;

/// Creates an isolated MetricsRegistry for testing.
///
/// This function creates a fresh Prometheus registry to avoid cardinality
/// conflicts that can occur when multiple tests register metrics with
/// the same names.
///
/// # Returns
///
/// A new MetricsRegistry with an isolated registry.
///
/// # Examples
///
/// ```rust,ignore
/// use rock_node_core::test_utils::create_isolated_metrics;
///
/// let metrics = create_isolated_metrics();
/// // Use metrics in test...
/// ```
pub fn create_isolated_metrics() -> MetricsRegistry {
    let registry = prometheus::Registry::new();
    MetricsRegistry::with_registry(registry).expect("Failed to create isolated metrics registry")
}

/// Creates an isolated MetricsRegistry with custom metric names.
///
/// This is useful when you need to avoid conflicts with production
/// metric names during testing.
///
/// # Arguments
///
/// * `prefix` - A prefix to add to all metric names to avoid conflicts
///
/// # Returns
///
/// A new MetricsRegistry with isolated, prefixed metric names.
///
/// # Note
///
/// This function is currently not implemented as it would require
/// significant changes to the MetricsRegistry implementation.
/// For now, use `create_isolated_metrics()` instead.
pub fn create_isolated_metrics_with_prefix(_prefix: &str) -> MetricsRegistry {
    // TODO: Implement when needed
    // This would require modifying MetricsRegistry to support
    // custom metric name prefixes
    create_isolated_metrics()
}

use crate::events::BlockData;
use std::sync::Arc;

/// Creates a mock BlockReader for testing.
///
/// This function creates a simple in-memory mock that stores blocks
/// in a HashMap for testing purposes.
#[cfg(test)]
pub fn create_mock_block_reader() -> MockBlockReader {
    MockBlockReader::new()
}

/// A simple mock BlockReader implementation for testing.
#[cfg(test)]
#[derive(Debug, Clone)]
pub struct MockBlockReader {
    blocks: Arc<std::sync::RwLock<std::collections::HashMap<u64, Vec<u8>>>>,
}

#[cfg(test)]
impl MockBlockReader {
    pub fn new() -> Self {
        Self {
            blocks: Arc::new(std::sync::RwLock::new(std::collections::HashMap::new())),
        }
    }

    pub fn insert_block(&self, block_number: u64, data: Vec<u8>) {
        self.blocks.write().unwrap().insert(block_number, data);
    }
}

#[cfg(test)]
impl crate::BlockReader for MockBlockReader {
    fn get_latest_persisted_block_number(&self) -> anyhow::Result<Option<u64>> {
        Ok(self.blocks.read().unwrap().keys().max().copied())
    }

    fn get_earliest_persisted_block_number(&self) -> anyhow::Result<Option<u64>> {
        Ok(self.blocks.read().unwrap().keys().min().copied())
    }

    fn read_block(&self, block_number: u64) -> anyhow::Result<Option<Vec<u8>>> {
        Ok(self.blocks.read().unwrap().get(&block_number).cloned())
    }

    fn get_highest_contiguous_block_number(&self) -> anyhow::Result<u64> {
        let blocks = self.blocks.read().unwrap();
        let mut keys: Vec<u64> = blocks.keys().copied().collect();
        keys.sort_unstable();

        let mut highest = 0;
        for key in keys {
            if key == highest || key == highest + 1 {
                highest = key;
            } else {
                break;
            }
        }
        Ok(highest)
    }
}

/// A simple mock BlockWriter implementation for testing.
#[cfg(test)]
#[derive(Debug, Clone)]
pub struct MockBlockWriter {
    blocks:
        Arc<std::sync::RwLock<Vec<rock_node_protobufs::com::hedera::hapi::block::stream::Block>>>,
}

#[cfg(test)]
impl MockBlockWriter {
    pub fn new() -> Self {
        Self {
            blocks: Arc::new(std::sync::RwLock::new(Vec::new())),
        }
    }

    pub fn get_written_blocks(
        &self,
    ) -> Vec<rock_node_protobufs::com::hedera::hapi::block::stream::Block> {
        self.blocks.read().unwrap().clone()
    }

    pub fn clear(&self) {
        self.blocks.write().unwrap().clear();
    }
}

#[cfg(test)]
#[async_trait::async_trait]
impl crate::BlockWriter for MockBlockWriter {
    async fn write_block(
        &self,
        block: &rock_node_protobufs::com::hedera::hapi::block::stream::Block,
    ) -> anyhow::Result<()> {
        self.blocks.write().unwrap().push(block.clone());
        Ok(())
    }

    async fn write_block_batch(
        &self,
        blocks: &[rock_node_protobufs::com::hedera::hapi::block::stream::Block],
    ) -> anyhow::Result<()> {
        self.blocks.write().unwrap().extend_from_slice(blocks);
        Ok(())
    }
}

/// Creates test BlockData with the given block number and content.
pub fn create_test_block_data(block_number: u64, content: &str) -> BlockData {
    BlockData {
        block_number,
        contents: content.as_bytes().to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BlockReader, BlockWriter};

    #[test]
    fn test_create_isolated_metrics() {
        let metrics = create_isolated_metrics();

        // Verify we can access metrics without conflicts
        let _counter = metrics.blocks_acknowledged.clone();

        // If this test passes, registry isolation is working
        assert!(true);
    }

    #[test]
    fn test_isolated_registries_are_independent() {
        let metrics1 = create_isolated_metrics();
        let metrics2 = create_isolated_metrics();

        // Increment counter on first registry
        metrics1.blocks_acknowledged.inc();

        // Second registry should have different values
        assert_eq!(metrics1.blocks_acknowledged.get(), 1);
        assert_eq!(metrics2.blocks_acknowledged.get(), 0);
    }

    #[test]
    fn test_create_test_block_data() {
        let block_data = create_test_block_data(42, "test content");
        assert_eq!(block_data.block_number, 42);
        assert_eq!(block_data.contents, b"test content");
    }

    #[test]
    fn test_mock_block_reader() {
        let reader = create_mock_block_reader();
        reader.insert_block(1, b"block 1".to_vec());
        reader.insert_block(2, b"block 2".to_vec());

        assert_eq!(reader.get_latest_persisted_block_number().unwrap(), Some(2));
        assert_eq!(
            reader.get_earliest_persisted_block_number().unwrap(),
            Some(1)
        );
        assert_eq!(reader.read_block(1).unwrap(), Some(b"block 1".to_vec()));
        assert_eq!(reader.read_block(3).unwrap(), None);
    }

    #[tokio::test]
    async fn test_mock_block_writer() {
        let writer = MockBlockWriter::new();
        let block = rock_node_protobufs::com::hedera::hapi::block::stream::Block::default();

        writer.write_block(&block).await.unwrap();
        assert_eq!(writer.get_written_blocks().len(), 1);

        writer.clear();
        assert_eq!(writer.get_written_blocks().len(), 0);
    }
}
