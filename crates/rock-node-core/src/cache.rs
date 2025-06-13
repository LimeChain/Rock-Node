use crate::events::BlockData;
use dashmap::DashMap;
use std::sync::Arc;
use uuid::Uuid;

/// A thread-safe, in-memory cache to hold block data temporarily
/// while it passes through the processing pipeline.
#[derive(Debug, Clone, Default)]
pub struct BlockDataCache {
    // DashMap is highly optimized for concurrent reads and writes.
    cache: Arc<DashMap<Uuid, BlockData>>,
}

impl BlockDataCache {
    pub fn new() -> Self {
        Self::default()
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

    /// Removes the block data from the cache, reclaiming memory.
    pub fn remove(&self, key: &Uuid) -> Option<BlockData> {
        self.cache.remove(key).map(|(_, data)| data)
    }
}
