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
