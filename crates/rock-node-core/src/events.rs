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

/// Published by the Persistence plugin after it has successfully saved the
/// block to disk.
#[derive(Debug, Clone, Copy)]
pub struct BlockPersisted {
    pub block_number: u64,
    pub cache_key: Uuid,
}
