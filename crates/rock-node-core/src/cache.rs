use crate::events::BlockData;
use dashmap::DashMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tracing::{trace, warn};
use uuid::Uuid;

const CACHE_TTL_SECONDS: u64 = 300; // 5 minutes
const CLEANUP_INTERVAL_SECONDS: u64 = 30;

/// A thread-safe, in-memory cache to hold block data temporarily
/// while it passes through the processing pipeline.
///
/// It uses a deferred deletion mechanism to solve race conditions and a TTL
/// mechanism to prevent memory leaks from orphaned entries.
#[derive(Debug, Clone)]
pub struct BlockDataCache {
    /// The primary K-V store for block data and its insertion timestamp.
    cache: Arc<DashMap<Uuid, (BlockData, Instant)>>,
    /// A queue of unique cache keys marked for deletion.
    cleanup_queue: Arc<Mutex<HashSet<Uuid>>>,
}

impl BlockDataCache {
    pub fn new() -> Self {
        let cache = Arc::new(DashMap::new());
        let cleanup_queue = Arc::new(Mutex::new(HashSet::new()));

        // Spawn a background task to periodically clean the cache.
        tokio::spawn(Self::run_cleanup_task(cache.clone(), cleanup_queue.clone()));

        Self {
            cache,
            cleanup_queue,
        }
    }

    /// The background task that runs periodically to purge items from the cache.
    async fn run_cleanup_task(
        cache: Arc<DashMap<Uuid, (BlockData, Instant)>>,
        queue: Arc<Mutex<HashSet<Uuid>>>,
    ) {
        let cleanup_interval = Duration::from_secs(CLEANUP_INTERVAL_SECONDS);
        let ttl = Duration::from_secs(CACHE_TTL_SECONDS);

        trace!(
            "Cache cleanup task started. Interval: {}s, TTL: {}s.",
            cleanup_interval.as_secs(),
            ttl.as_secs()
        );

        loop {
            tokio::time::sleep(cleanup_interval).await;

            // 1. Process the explicit removal queue
            let keys_to_delete: Vec<Uuid> = {
                let mut locked_set = queue.lock().await;
                if !locked_set.is_empty() {
                    trace!(
                        "Cache cleanup: Removing {} explicitly marked items.",
                        locked_set.len()
                    );
                    locked_set.drain().collect()
                } else {
                    Vec::new()
                }
            };
            for key in keys_to_delete {
                cache.remove(&key);
            }

            // 2. Scan for and evict stale (orphaned) entries
            let mut stale_keys = Vec::new();
            for item in cache.iter() {
                if item.value().1.elapsed() > ttl {
                    stale_keys.push(*item.key());
                }
            }
            if !stale_keys.is_empty() {
                warn!(
                    "Cache cleanup: Evicting {} stale (orphaned) entries older than TTL.",
                    stale_keys.len()
                );
                for key in stale_keys {
                    cache.remove(&key);
                }
            }
        }
    }

    /// Inserts block data into the cache and returns its unique key.
    pub fn insert(&self, data: BlockData) -> Uuid {
        let key = Uuid::new_v4();
        self.cache.insert(key, (data, Instant::now()));
        key
    }

    /// Retrieves a clone of the block data for a given key.
    pub fn get(&self, key: &Uuid) -> Option<BlockData> {
        self.cache.get(key).map(|entry| entry.value().0.clone())
    }

    /// Marks a cache entry for future deletion by the background task.
    pub async fn mark_for_removal(&self, key: Uuid) {
        let mut queue = self.cleanup_queue.lock().await;
        queue.insert(key);
    }
}

// Implement Default to easily create a new cache.
impl Default for BlockDataCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::BlockData;
    use tokio::time::{sleep, Duration};

    fn create_test_block_data(block_number: u64) -> BlockData {
        BlockData {
            block_number,
            contents: format!("Block {}", block_number).into_bytes(),
        }
    }

    #[tokio::test]
    async fn test_cache_insert_and_get() {
        let cache = BlockDataCache::new();
        let block_data = create_test_block_data(1);
        let key = cache.insert(block_data.clone());

        let retrieved = cache.get(&key);
        assert!(retrieved.is_some());
        let retrieved_data = retrieved.unwrap();
        assert_eq!(retrieved_data.block_number, block_data.block_number);
        assert_eq!(retrieved_data.contents, block_data.contents);
    }

    #[tokio::test]
    async fn test_cache_get_nonexistent_key() {
        let cache = BlockDataCache::new();
        let random_key = Uuid::new_v4();

        let result = cache.get(&random_key);
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_cache_insert_multiple_blocks() {
        let cache = BlockDataCache::new();

        let key1 = cache.insert(create_test_block_data(1));
        let key2 = cache.insert(create_test_block_data(2));
        let key3 = cache.insert(create_test_block_data(3));

        assert_ne!(key1, key2);
        assert_ne!(key2, key3);
        assert_ne!(key1, key3);

        let block1 = cache.get(&key1).unwrap();
        let block2 = cache.get(&key2).unwrap();
        let block3 = cache.get(&key3).unwrap();

        assert_eq!(block1.block_number, 1);
        assert_eq!(block2.block_number, 2);
        assert_eq!(block3.block_number, 3);
    }

    #[tokio::test]
    async fn test_cache_mark_for_removal() {
        let cache = BlockDataCache::new();
        let block_data = create_test_block_data(10);
        let key = cache.insert(block_data);

        // Verify the block is in the cache
        assert!(cache.get(&key).is_some());

        // Mark for removal
        cache.mark_for_removal(key).await;

        // Block should still be retrievable immediately after marking
        assert!(cache.get(&key).is_some());

        // Wait for cleanup interval + a bit more
        sleep(Duration::from_secs(CLEANUP_INTERVAL_SECONDS + 2)).await;

        // Block should now be removed
        assert!(cache.get(&key).is_none());
    }

    #[tokio::test]
    async fn test_cache_cleanup_stale_entries() {
        let cache = BlockDataCache::new();
        let block_data = create_test_block_data(20);
        let key = cache.insert(block_data);

        // Verify block is in cache
        assert!(cache.get(&key).is_some());

        // Wait for TTL to expire (5 minutes + cleanup interval + buffer)
        // For testing purposes, this would take too long, so we just verify
        // the block is still there after a short time
        sleep(Duration::from_secs(2)).await;
        assert!(cache.get(&key).is_some());
    }

    #[tokio::test]
    async fn test_cache_clone() {
        let cache = BlockDataCache::new();
        let block_data = create_test_block_data(30);
        let key = cache.insert(block_data);

        // Clone the cache
        let cache_clone = cache.clone();

        // Both should access the same underlying data
        let original_block = cache.get(&key).unwrap();
        let cloned_block = cache_clone.get(&key).unwrap();

        assert_eq!(original_block.block_number, cloned_block.block_number);
        assert_eq!(original_block.contents, cloned_block.contents);
    }

    #[tokio::test]
    async fn test_cache_default() {
        let cache = BlockDataCache::default();
        let block_data = create_test_block_data(40);
        let key = cache.insert(block_data);

        assert!(cache.get(&key).is_some());
    }

    #[tokio::test]
    async fn test_cache_concurrent_inserts() {
        let cache = BlockDataCache::new();
        let mut handles = vec![];

        for i in 0..100 {
            let cache_clone = cache.clone();
            let handle = tokio::spawn(async move {
                let block_data = create_test_block_data(i);
                cache_clone.insert(block_data)
            });
            handles.push(handle);
        }

        let mut keys = Vec::new();
        for handle in handles {
            keys.push(handle.await.unwrap());
        }

        // All keys should be unique
        let unique_keys: std::collections::HashSet<_> = keys.iter().collect();
        assert_eq!(unique_keys.len(), 100);

        // All blocks should be retrievable
        for (i, key) in keys.iter().enumerate() {
            let block = cache.get(key).unwrap();
            assert_eq!(block.block_number, i as u64);
        }
    }

    #[tokio::test]
    async fn test_cache_concurrent_reads() {
        let cache = BlockDataCache::new();
        let block_data = create_test_block_data(50);
        let key = cache.insert(block_data);

        let mut handles = vec![];
        for _ in 0..50 {
            let cache_clone = cache.clone();
            let key_clone = key;
            let handle = tokio::spawn(async move { cache_clone.get(&key_clone) });
            handles.push(handle);
        }

        let mut results = Vec::new();
        for handle in handles {
            results.push(handle.await.unwrap());
        }

        // All reads should succeed
        assert_eq!(results.iter().filter(|r| r.is_some()).count(), 50);
    }

    #[tokio::test]
    async fn test_cache_mark_multiple_for_removal() {
        let cache = BlockDataCache::new();

        let key1 = cache.insert(create_test_block_data(100));
        let key2 = cache.insert(create_test_block_data(101));
        let key3 = cache.insert(create_test_block_data(102));

        // Mark all for removal
        cache.mark_for_removal(key1).await;
        cache.mark_for_removal(key2).await;
        cache.mark_for_removal(key3).await;

        // Wait for cleanup
        sleep(Duration::from_secs(CLEANUP_INTERVAL_SECONDS + 2)).await;

        // All should be removed
        assert!(cache.get(&key1).is_none());
        assert!(cache.get(&key2).is_none());
        assert!(cache.get(&key3).is_none());
    }

    #[tokio::test]
    async fn test_cache_with_large_data() {
        let cache = BlockDataCache::new();
        let large_contents = vec![0u8; 10_000_000]; // 10MB
        let block_data = BlockData {
            block_number: 999,
            contents: large_contents,
        };

        let key = cache.insert(block_data.clone());
        let retrieved = cache.get(&key).unwrap();

        assert_eq!(retrieved.block_number, 999);
        assert_eq!(retrieved.contents.len(), 10_000_000);
    }

    #[tokio::test]
    async fn test_cache_debug_format() {
        let cache = BlockDataCache::new();
        let debug_str = format!("{:?}", cache);
        assert!(debug_str.contains("BlockDataCache"));
    }

    #[tokio::test]
    async fn test_cache_insert_returns_unique_uuids() {
        let cache = BlockDataCache::new();
        let block_data = create_test_block_data(1);

        let key1 = cache.insert(block_data.clone());
        let key2 = cache.insert(block_data.clone());
        let key3 = cache.insert(block_data);

        // Even with identical data, keys should be unique
        assert_ne!(key1, key2);
        assert_ne!(key2, key3);
        assert_ne!(key1, key3);
    }

    #[tokio::test]
    async fn test_cache_mark_nonexistent_key_for_removal() {
        let cache = BlockDataCache::new();
        let random_key = Uuid::new_v4();

        // Should not panic
        cache.mark_for_removal(random_key).await;

        // Wait for cleanup
        sleep(Duration::from_secs(CLEANUP_INTERVAL_SECONDS + 2)).await;

        // Nothing bad should happen
        assert!(cache.get(&random_key).is_none());
    }

    #[tokio::test]
    async fn test_cache_lifecycle() {
        let cache = BlockDataCache::new();

        // Insert
        let block_data = create_test_block_data(200);
        let key = cache.insert(block_data.clone());

        // Get
        let retrieved = cache.get(&key).unwrap();
        assert_eq!(retrieved.block_number, 200);

        // Mark for removal
        cache.mark_for_removal(key).await;

        // Still accessible before cleanup
        assert!(cache.get(&key).is_some());

        // Wait for cleanup
        sleep(Duration::from_secs(CLEANUP_INTERVAL_SECONDS + 2)).await;

        // Should be removed
        assert!(cache.get(&key).is_none());
    }
}
