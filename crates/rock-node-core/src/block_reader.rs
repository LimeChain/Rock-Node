use anyhow::Result;
use std::fmt::Debug;
use std::sync::Arc;

pub trait BlockReader: Debug + Send + Sync + 'static {
    fn get_latest_persisted_block_number(&self) -> Result<Option<u64>>;
    fn get_earliest_persisted_block_number(&self) -> Result<Option<u64>>;
    fn read_block(&self, block_number: u64) -> Result<Option<Vec<u8>>>;
    fn get_highest_contiguous_block_number(&self) -> Result<u64>;
}

/// A concrete, shareable handle for the BlockReader service.
///
/// This provider is registered in the `AppContext` by the Persistence Plugin at startup.
/// Other plugins can then request this provider to get access to the `BlockReader` service.
#[derive(Clone, Debug)]
pub struct BlockReaderProvider {
    reader: Arc<dyn BlockReader>,
}

impl BlockReaderProvider {
    pub fn new(reader: Arc<dyn BlockReader>) -> Self {
        Self { reader }
    }

    pub fn get_reader(&self) -> Arc<dyn BlockReader> {
        self.reader.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::RwLock;

    /// A simple mock implementation of BlockReader for testing
    #[derive(Debug)]
    struct MockBlockReader {
        blocks: RwLock<HashMap<u64, Vec<u8>>>,
    }

    impl MockBlockReader {
        fn new() -> Self {
            Self {
                blocks: RwLock::new(HashMap::new()),
            }
        }

        fn insert_block(&self, block_number: u64, data: Vec<u8>) {
            self.blocks.write().unwrap().insert(block_number, data);
        }
    }

    impl BlockReader for MockBlockReader {
        fn get_latest_persisted_block_number(&self) -> Result<Option<u64>> {
            Ok(self.blocks.read().unwrap().keys().max().copied())
        }

        fn get_earliest_persisted_block_number(&self) -> Result<Option<u64>> {
            Ok(self.blocks.read().unwrap().keys().min().copied())
        }

        fn read_block(&self, block_number: u64) -> Result<Option<Vec<u8>>> {
            Ok(self.blocks.read().unwrap().get(&block_number).cloned())
        }

        fn get_highest_contiguous_block_number(&self) -> Result<u64> {
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

    #[test]
    fn test_block_reader_provider_creation() {
        let mock_reader = Arc::new(MockBlockReader::new());
        let provider = BlockReaderProvider::new(mock_reader);
        assert!(format!("{:?}", provider).contains("BlockReaderProvider"));
    }

    #[test]
    fn test_block_reader_provider_get_reader() {
        let mock_reader = Arc::new(MockBlockReader::new());
        let provider = BlockReaderProvider::new(mock_reader.clone());

        let reader = provider.get_reader();
        assert!(reader
            .get_latest_persisted_block_number()
            .unwrap()
            .is_none());
    }

    #[test]
    fn test_block_reader_provider_clone() {
        let mock_reader = Arc::new(MockBlockReader::new());
        mock_reader.insert_block(1, b"block 1".to_vec());

        let provider = BlockReaderProvider::new(mock_reader);
        let provider_clone = provider.clone();

        let reader1 = provider.get_reader();
        let reader2 = provider_clone.get_reader();

        assert_eq!(
            reader1.get_latest_persisted_block_number().unwrap(),
            reader2.get_latest_persisted_block_number().unwrap()
        );
    }

    #[test]
    fn test_mock_block_reader_empty() {
        let reader = MockBlockReader::new();
        assert_eq!(reader.get_latest_persisted_block_number().unwrap(), None);
        assert_eq!(reader.get_earliest_persisted_block_number().unwrap(), None);
        assert_eq!(reader.read_block(1).unwrap(), None);
        assert_eq!(reader.get_highest_contiguous_block_number().unwrap(), 0);
    }

    #[test]
    fn test_mock_block_reader_single_block() {
        let reader = MockBlockReader::new();
        reader.insert_block(1, b"block 1".to_vec());

        assert_eq!(reader.get_latest_persisted_block_number().unwrap(), Some(1));
        assert_eq!(
            reader.get_earliest_persisted_block_number().unwrap(),
            Some(1)
        );
        assert_eq!(reader.read_block(1).unwrap(), Some(b"block 1".to_vec()));
        assert_eq!(reader.get_highest_contiguous_block_number().unwrap(), 1);
    }

    #[test]
    fn test_mock_block_reader_multiple_blocks() {
        let reader = MockBlockReader::new();
        reader.insert_block(1, b"block 1".to_vec());
        reader.insert_block(2, b"block 2".to_vec());
        reader.insert_block(3, b"block 3".to_vec());

        assert_eq!(reader.get_latest_persisted_block_number().unwrap(), Some(3));
        assert_eq!(
            reader.get_earliest_persisted_block_number().unwrap(),
            Some(1)
        );
        assert_eq!(reader.read_block(2).unwrap(), Some(b"block 2".to_vec()));
        assert_eq!(reader.get_highest_contiguous_block_number().unwrap(), 3);
    }

    #[test]
    fn test_mock_block_reader_gap_in_blocks() {
        let reader = MockBlockReader::new();
        reader.insert_block(1, b"block 1".to_vec());
        reader.insert_block(2, b"block 2".to_vec());
        reader.insert_block(5, b"block 5".to_vec());

        assert_eq!(reader.get_latest_persisted_block_number().unwrap(), Some(5));
        assert_eq!(
            reader.get_earliest_persisted_block_number().unwrap(),
            Some(1)
        );
        assert_eq!(reader.get_highest_contiguous_block_number().unwrap(), 2);
    }

    #[test]
    fn test_mock_block_reader_nonexistent_block() {
        let reader = MockBlockReader::new();
        reader.insert_block(1, b"block 1".to_vec());

        assert_eq!(reader.read_block(999).unwrap(), None);
    }

    #[test]
    fn test_mock_block_reader_highest_contiguous_with_gap_at_start() {
        let reader = MockBlockReader::new();
        reader.insert_block(5, b"block 5".to_vec());
        reader.insert_block(6, b"block 6".to_vec());
        reader.insert_block(7, b"block 7".to_vec());

        // Since there's no block 0, highest contiguous should be 0
        assert_eq!(reader.get_highest_contiguous_block_number().unwrap(), 0);
    }

    #[test]
    fn test_mock_block_reader_highest_contiguous_starting_from_zero() {
        let reader = MockBlockReader::new();
        reader.insert_block(0, b"block 0".to_vec());
        reader.insert_block(1, b"block 1".to_vec());
        reader.insert_block(2, b"block 2".to_vec());

        assert_eq!(reader.get_highest_contiguous_block_number().unwrap(), 2);
    }

    #[test]
    fn test_block_reader_trait_send_sync() {
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}
        assert_send::<Arc<dyn BlockReader>>();
        assert_sync::<Arc<dyn BlockReader>>();
    }

    #[test]
    fn test_provider_debug_format() {
        let mock_reader = Arc::new(MockBlockReader::new());
        let provider = BlockReaderProvider::new(mock_reader);
        let debug_str = format!("{:?}", provider);
        assert!(debug_str.contains("BlockReaderProvider"));
    }

    #[test]
    fn test_provider_shares_underlying_reader() {
        let mock_reader = Arc::new(MockBlockReader::new());
        mock_reader.insert_block(100, b"shared block".to_vec());

        let provider1 = BlockReaderProvider::new(mock_reader.clone());
        let provider2 = BlockReaderProvider::new(mock_reader);

        let reader1 = provider1.get_reader();
        let reader2 = provider2.get_reader();

        // Both should see the same data
        assert_eq!(
            reader1.read_block(100).unwrap(),
            reader2.read_block(100).unwrap()
        );
    }
}
