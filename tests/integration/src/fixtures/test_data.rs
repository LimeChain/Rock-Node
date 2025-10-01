use rock_node_core::events::{BlockData, BlockItemsReceived, BlockPersisted, BlockVerified};
use rock_node_protobufs::com::hedera::hapi::block::stream::{Block, BlockItem};
use uuid::Uuid;

/// Builder for creating test data
pub struct TestDataBuilder {
    start_block_number: u64,
}

impl Default for TestDataBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl TestDataBuilder {
    pub fn new() -> Self {
        Self {
            start_block_number: 1,
        }
    }

    pub fn with_start_block_number(mut self, start: u64) -> Self {
        self.start_block_number = start;
        self
    }

    /// Create a single test block with a valid BlockHeader
    pub fn create_block(&self, block_number: u64) -> Block {
        use rock_node_protobufs::com::hedera::hapi::block::stream::block_item::Item as BlockItemType;
        use rock_node_protobufs::com::hedera::hapi::block::stream::output::BlockHeader;
        use rock_node_protobufs::proto::SemanticVersion;

        let header = BlockHeader {
            hapi_proto_version: Some(SemanticVersion {
                major: 0,
                minor: 45,
                patch: 1,
                pre: "test".to_string(),
                build: "integration".to_string(),
            }),
            software_version: Some(SemanticVersion {
                major: 0,
                minor: 45,
                patch: 1,
                pre: "test".to_string(),
                build: "integration".to_string(),
            }),
            number: block_number,
            block_timestamp: None,
            hash_algorithm: 0,
        };

        let header_item = BlockItem {
            item: Some(BlockItemType::BlockHeader(header)),
        };

        Block {
            items: vec![header_item],
            ..Default::default()
        }
    }

    /// Create a batch of sequential blocks
    pub fn create_block_batch(&self, count: u64) -> Vec<Block> {
        (0..count)
            .map(|i| self.create_block(self.start_block_number + i))
            .collect()
    }

    /// Create test BlockData for the cache
    pub fn create_block_data(&self, block_number: u64) -> BlockData {
        BlockData {
            block_number,
            contents: format!("test_block_{}", block_number).into_bytes(),
        }
    }

    /// Create test BlockData with custom content
    pub fn create_block_data_with_content(&self, block_number: u64, content: Vec<u8>) -> BlockData {
        BlockData {
            block_number,
            contents: content,
        }
    }

    /// Create a BlockItemsReceived event
    pub fn create_block_items_received_event(
        &self,
        block_number: u64,
        cache_key: Option<Uuid>,
    ) -> BlockItemsReceived {
        BlockItemsReceived {
            block_number,
            cache_key: cache_key.unwrap_or_else(Uuid::new_v4),
        }
    }

    /// Create a BlockVerified event
    pub fn create_block_verified_event(
        &self,
        block_number: u64,
        cache_key: Option<Uuid>,
    ) -> BlockVerified {
        BlockVerified {
            block_number,
            cache_key: cache_key.unwrap_or_else(Uuid::new_v4),
        }
    }

    /// Create a BlockPersisted event
    pub fn create_block_persisted_event(
        &self,
        block_number: u64,
        cache_key: Option<Uuid>,
    ) -> BlockPersisted {
        BlockPersisted {
            block_number,
            cache_key: cache_key.unwrap_or_else(Uuid::new_v4),
        }
    }
}

/// Helper functions for common test data creation
pub fn create_test_block(block_number: u64) -> Block {
    TestDataBuilder::new().create_block(block_number)
}

pub fn create_test_block_batch(start: u64, count: u64) -> Vec<Block> {
    TestDataBuilder::new()
        .with_start_block_number(start)
        .create_block_batch(count)
}

pub fn create_test_block_data(block_number: u64) -> BlockData {
    TestDataBuilder::new().create_block_data(block_number)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_single_block() {
        let block = create_test_block(42);
        assert!(!block.items.is_empty());
    }

    #[test]
    fn test_create_block_batch() {
        let blocks = create_test_block_batch(1, 10);
        assert_eq!(blocks.len(), 10);
    }

    #[test]
    fn test_create_block_data() {
        let block_data = create_test_block_data(100);
        assert_eq!(block_data.block_number, 100);
        assert_eq!(
            String::from_utf8(block_data.contents).unwrap(),
            "test_block_100"
        );
    }

    #[test]
    fn test_builder_with_custom_start() {
        let builder = TestDataBuilder::new().with_start_block_number(1000);
        let blocks = builder.create_block_batch(5);
        assert_eq!(blocks.len(), 5);
    }

    #[test]
    fn test_create_events() {
        let builder = TestDataBuilder::new();
        let uuid = Uuid::new_v4();

        let items_received = builder.create_block_items_received_event(1, Some(uuid));
        assert_eq!(items_received.block_number, 1);
        assert_eq!(items_received.cache_key, uuid);

        let verified = builder.create_block_verified_event(1, Some(uuid));
        assert_eq!(verified.block_number, 1);
        assert_eq!(verified.cache_key, uuid);

        let persisted = builder.create_block_persisted_event(1, Some(uuid));
        assert_eq!(persisted.block_number, 1);
        assert_eq!(persisted.cache_key, uuid);
    }

    #[test]
    fn test_create_events_with_auto_uuid() {
        let builder = TestDataBuilder::new();

        let event1 = builder.create_block_items_received_event(1, None);
        let event2 = builder.create_block_items_received_event(1, None);

        // Each should have a different UUID
        assert_ne!(event1.cache_key, event2.cache_key);
    }
}
