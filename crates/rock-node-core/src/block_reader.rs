/// The service trait for reading block data from persistent storage.
pub trait BlockReader: Send + Sync {
    /// Returns the latest (highest number) block number persisted, or -1 if none.
    fn get_latest_persisted_block_number(&self) -> i64;

    /// Returns the earliest (lowest number) block number available in hot storage, or -1 if none.
    fn get_earliest_persisted_block_number(&self) -> i64;
}
