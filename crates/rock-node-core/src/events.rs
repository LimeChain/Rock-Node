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

/// Represents a block after filtering logic has been applied.
/// It could be a slimmed-down version of the original block.
#[derive(Debug, Clone)]
pub struct FilteredBlock {
    pub block_number: u64,
    // In the future, this could contain a Vec of specific, filtered items.
    // For now, we just signal that the block was processed.
}

/// Published by the Ingress Plugin's filter task after processing a block.
#[derive(Debug, Clone)]
pub struct FilteredBlockReady {
    pub filtered_block: FilteredBlock,
}
