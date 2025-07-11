use crate::events::BlockData;
use dashmap::DashMap;
use std::collections::HashSet; // <-- Import HashSet
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::trace;
use uuid::Uuid;

/// A thread-safe, in-memory cache to hold block data temporarily
/// while it passes through the processing pipeline.
///
/// It uses a deferred deletion mechanism to solve race conditions where multiple
/// plugins may need to access the same cached data after an event is broadcast.
#[derive(Debug, Clone)]
pub struct BlockDataCache {
    /// The primary K-V store for block data.
    cache: Arc<DashMap<Uuid, BlockData>>,
    /// A queue of unique cache keys marked for deletion.
    /// Using a HashSet automatically handles duplicate requests to remove the same key.
    cleanup_queue: Arc<Mutex<HashSet<Uuid>>>,
}

impl BlockDataCache {
    /// Creates a new `BlockDataCache` and spawns its background cleanup task.
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
        cache: Arc<DashMap<Uuid, BlockData>>,
        queue: Arc<Mutex<HashSet<Uuid>>>,
    ) {
        let cleanup_interval = Duration::from_secs(10);
        trace!(
            "Cache cleanup task started. Will process queue every {} seconds.",
            cleanup_interval.as_secs()
        );

        loop {
            tokio::time::sleep(cleanup_interval).await;

            let keys_to_delete: Vec<Uuid> = {
                let mut locked_set = queue.lock().await;
                if locked_set.is_empty() {
                    continue;
                }
                // Draining the set is an efficient way to move all items out.
                locked_set.drain().collect()
            };

            if !keys_to_delete.is_empty() {
                trace!("Cache cleanup: Removing {} items.", keys_to_delete.len());
                for key in keys_to_delete {
                    cache.remove(&key);
                }
            }
        }
    }

    /// Inserts block data into the cache and returns its unique key.
    pub fn insert(&self, data: BlockData) -> Uuid {
        let key = Uuid::new_v4();
        self.cache.insert(key, data);
        key
    }

    /// Retrieves a clone of the block data for a given key.
    pub fn get(&self, key: &Uuid) -> Option<BlockData> {
        self.cache.get(key).map(|entry| entry.value().clone())
    }

    /// Marks a cache entry for future deletion by the background task.
    ///
    /// This is idempotent; calling it multiple times for the same key has no
    /// additional effect.
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
