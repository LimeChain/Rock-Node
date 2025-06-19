use anyhow::{anyhow, Result};
use rock_node_core::database::CF_METADATA;
use rocksdb::{WriteBatch, DB};
use std::sync::Arc;

// Define the keys we use for our metadata as constants.
const LATEST_PERSISTED_KEY: &[u8] = b"latest_persisted";
const EARLIEST_HOT_KEY: &[u8] = b"earliest_hot";
const HIGHEST_CONTIGUOUS_KEY: &[u8] = b"highest_contiguous";

/// Manages all metadata state within the 'metadata' Column Family of RocksDB.
#[derive(Debug, Clone)]
pub struct StateManager {
    db: Arc<DB>,
}

impl StateManager {
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    /// Retrieves a u64 value from the metadata column family. Returns 0 if not found.
    fn get_u64(&self, key: &[u8]) -> Result<u64> {
        let cf = self
            .db
            .cf_handle(CF_METADATA)
            .ok_or_else(|| anyhow!("Could not get handle for CF: {}", CF_METADATA))?;

        let value = self.db.get_cf(cf, key)?.unwrap_or_default();
        Ok(u64::from_be_bytes(
            value.try_into().unwrap_or([0; 8]),
        ))
    }

    /// A helper to add a u64 write to a batch operation.
    fn set_u64(&self, key: &[u8], value: u64, batch: &mut WriteBatch) -> Result<()> {
        let cf = self
            .db
            .cf_handle(CF_METADATA)
            .ok_or_else(|| anyhow!("Could not get handle for CF: {}", CF_METADATA))?;
        batch.put_cf(cf, key, &value.to_be_bytes());
        Ok(())
    }

    pub fn get_latest_persisted(&self) -> Result<u64> {
        self.get_u64(LATEST_PERSISTED_KEY)
    }

    pub fn get_earliest_hot(&self) -> Result<u64> {
        self.get_u64(EARLIEST_HOT_KEY)
    }

    pub fn get_highest_contiguous(&self) -> Result<u64> {
        self.get_u64(HIGHEST_CONTIGUOUS_KEY)
    }

    pub fn set_latest_persisted(&self, block_number: u64, batch: &mut WriteBatch) -> Result<()> {
        self.set_u64(LATEST_PERSISTED_KEY, block_number, batch)
    }

    pub fn set_earliest_hot(&self, block_number: u64, batch: &mut WriteBatch) -> Result<()> {
        self.set_u64(EARLIEST_HOT_KEY, block_number, batch)
    }
    
    pub fn set_highest_contiguous(&self, block_number: u64, batch: &mut WriteBatch) -> Result<()> {
        self.set_u64(HIGHEST_CONTIGUOUS_KEY, block_number, batch)
    }

    /// Initializes the earliest_hot key if it doesn't exist.
    /// This should be called when the first block is ever written.
    pub fn initialize_earliest_hot(&self, block_number: u64, batch: &mut WriteBatch) -> Result<()> {
        let current_earliest = self.get_earliest_hot()?;
        if current_earliest == 0 {
            self.set_earliest_hot(block_number, batch)?;
        }
        Ok(())
    }
}
