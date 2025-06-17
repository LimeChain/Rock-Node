
use anyhow::Result;
use std::fmt::Debug;
use std::sync::Arc;

pub trait BlockReader: Debug + Send + Sync + 'static {
    fn get_latest_persisted_block_number(&self) -> i64;
    fn get_earliest_persisted_block_number(&self) -> i64;
    fn read_block(&self, block_number: u64) -> Result<Option<Vec<u8>>>;
}


#[derive(Clone, Debug)]
pub struct BlockReaderProvider {
    reader: Arc<dyn BlockReader>,
}

impl BlockReaderProvider {
    pub fn new(reader: Arc<dyn BlockReader>) -> Self {
        Self { reader }
    }

    /// This is the method the compiler was looking for.
    pub fn get_reader(&self) -> Arc<dyn BlockReader> {
        self.reader.clone()
    }
}
