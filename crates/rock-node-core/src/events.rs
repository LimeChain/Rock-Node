use uuid::Uuid;

/// A placeholder for the full block data that will be stored in the cache.
#[derive(Debug, Clone)]
pub struct BlockData {
    pub block_number: u64,
    pub contents: Vec<u8>, // We'll use a simple string for now
}

/// Published by the Publish plugin after it has received a complete block
/// and stored it in the BlockDataCache.
#[derive(Debug, Clone, Copy)]
pub struct BlockItemsReceived {
    pub block_number: u64,
    pub cache_key: Uuid,
}

/// Published by the Verifier plugin after it has fetched the block data
/// and successfully verified it.
#[derive(Debug, Clone, Copy)]
pub struct BlockVerified {
    pub block_number: u64,
    pub cache_key: Uuid,
}

/// Published by the Verifier plugin when block verification fails.
#[derive(Debug, Clone)]
pub struct BlockVerificationFailed {
    pub block_number: u64,
    pub cache_key: Uuid,
    pub reason: String,
}

/// Published by the Persistence plugin after it has successfully saved the
/// block to disk.
#[derive(Debug, Clone, Copy)]
pub struct BlockPersisted {
    pub block_number: u64,
    pub cache_key: Uuid,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_block_data_creation() {
        let data = BlockData {
            block_number: 42,
            contents: vec![1, 2, 3, 4],
        };
        assert_eq!(data.block_number, 42);
        assert_eq!(data.contents, vec![1, 2, 3, 4]);
    }

    #[test]
    fn test_block_data_clone() {
        let data = BlockData {
            block_number: 100,
            contents: b"test data".to_vec(),
        };
        let cloned = data.clone();
        assert_eq!(data.block_number, cloned.block_number);
        assert_eq!(data.contents, cloned.contents);
    }

    #[test]
    fn test_block_data_with_empty_contents() {
        let data = BlockData {
            block_number: 0,
            contents: vec![],
        };
        assert_eq!(data.block_number, 0);
        assert!(data.contents.is_empty());
    }

    #[test]
    fn test_block_data_with_large_contents() {
        let large_data = vec![0u8; 1_000_000]; // 1MB
        let data = BlockData {
            block_number: 999,
            contents: large_data.clone(),
        };
        assert_eq!(data.block_number, 999);
        assert_eq!(data.contents.len(), 1_000_000);
    }

    #[test]
    fn test_block_data_debug_format() {
        let data = BlockData {
            block_number: 5,
            contents: vec![1, 2, 3],
        };
        let debug_str = format!("{:?}", data);
        assert!(debug_str.contains("BlockData"));
        assert!(debug_str.contains("block_number"));
        assert!(debug_str.contains("contents"));
    }

    #[test]
    fn test_block_items_received_creation() {
        let uuid = Uuid::new_v4();
        let event = BlockItemsReceived {
            block_number: 10,
            cache_key: uuid,
        };
        assert_eq!(event.block_number, 10);
        assert_eq!(event.cache_key, uuid);
    }

    #[test]
    fn test_block_items_received_clone() {
        let uuid = Uuid::new_v4();
        let event = BlockItemsReceived {
            block_number: 20,
            cache_key: uuid,
        };
        let cloned = event;
        assert_eq!(event.block_number, cloned.block_number);
        assert_eq!(event.cache_key, cloned.cache_key);
    }

    #[test]
    fn test_block_items_received_copy() {
        let uuid = Uuid::new_v4();
        let event = BlockItemsReceived {
            block_number: 30,
            cache_key: uuid,
        };
        let copied = event; // Copy, not clone
        assert_eq!(event.block_number, copied.block_number);
        assert_eq!(event.cache_key, copied.cache_key);
    }

    #[test]
    fn test_block_verified_creation() {
        let uuid = Uuid::new_v4();
        let event = BlockVerified {
            block_number: 50,
            cache_key: uuid,
        };
        assert_eq!(event.block_number, 50);
        assert_eq!(event.cache_key, uuid);
    }

    #[test]
    fn test_block_verified_debug_format() {
        let uuid = Uuid::new_v4();
        let event = BlockVerified {
            block_number: 60,
            cache_key: uuid,
        };
        let debug_str = format!("{:?}", event);
        assert!(debug_str.contains("BlockVerified"));
        assert!(debug_str.contains("block_number"));
        assert!(debug_str.contains("cache_key"));
    }

    #[test]
    fn test_block_persisted_creation() {
        let uuid = Uuid::new_v4();
        let event = BlockPersisted {
            block_number: 70,
            cache_key: uuid,
        };
        assert_eq!(event.block_number, 70);
        assert_eq!(event.cache_key, uuid);
    }

    #[test]
    fn test_block_persisted_clone() {
        let uuid = Uuid::new_v4();
        let event = BlockPersisted {
            block_number: 80,
            cache_key: uuid,
        };
        let cloned = event;
        assert_eq!(event.block_number, cloned.block_number);
        assert_eq!(event.cache_key, cloned.cache_key);
    }

    #[test]
    fn test_all_events_with_same_uuid() {
        let uuid = Uuid::new_v4();
        let block_number = 100;

        let received = BlockItemsReceived {
            block_number,
            cache_key: uuid,
        };
        let verified = BlockVerified {
            block_number,
            cache_key: uuid,
        };
        let persisted = BlockPersisted {
            block_number,
            cache_key: uuid,
        };

        assert_eq!(received.cache_key, verified.cache_key);
        assert_eq!(verified.cache_key, persisted.cache_key);
        assert_eq!(received.block_number, verified.block_number);
        assert_eq!(verified.block_number, persisted.block_number);
    }

    #[test]
    fn test_events_send_sync() {
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}

        assert_send::<BlockData>();
        assert_sync::<BlockData>();
        assert_send::<BlockItemsReceived>();
        assert_sync::<BlockItemsReceived>();
        assert_send::<BlockVerified>();
        assert_sync::<BlockVerified>();
        assert_send::<BlockPersisted>();
        assert_sync::<BlockPersisted>();
    }

    #[test]
    fn test_event_lifecycle() {
        // Simulate the event lifecycle through the pipeline
        let uuid = Uuid::new_v4();
        let block_number = 200;

        // Step 1: Block items received
        let received = BlockItemsReceived {
            block_number,
            cache_key: uuid,
        };
        assert_eq!(received.block_number, block_number);

        // Step 2: Block verified
        let verified = BlockVerified {
            block_number: received.block_number,
            cache_key: received.cache_key,
        };
        assert_eq!(verified.block_number, received.block_number);
        assert_eq!(verified.cache_key, received.cache_key);

        // Step 3: Block persisted
        let persisted = BlockPersisted {
            block_number: verified.block_number,
            cache_key: verified.cache_key,
        };
        assert_eq!(persisted.block_number, verified.block_number);
        assert_eq!(persisted.cache_key, verified.cache_key);
    }

    #[test]
    fn test_block_data_send_across_threads() {
        use std::sync::{Arc, Mutex};
        use std::thread;

        let data = Arc::new(Mutex::new(BlockData {
            block_number: 1,
            contents: b"thread test".to_vec(),
        }));

        let data_clone = Arc::clone(&data);
        let handle = thread::spawn(move || {
            let d = data_clone.lock().unwrap();
            d.block_number
        });

        let result = handle.join().unwrap();
        assert_eq!(result, 1);
    }
}
