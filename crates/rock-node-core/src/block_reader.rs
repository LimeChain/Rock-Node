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
