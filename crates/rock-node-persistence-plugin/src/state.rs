use anyhow::{anyhow, Result};
use rock_node_core::database::CF_METADATA;
use rocksdb::{WriteBatch, DB};
use std::sync::Arc;

const LATEST_PERSISTED_KEY: &[u8] = b"latest_persisted";
const EARLIEST_HOT_KEY: &[u8] = b"earliest_hot";
const HIGHEST_CONTIGUOUS_KEY: &[u8] = b"highest_contiguous";
const TRUE_EARLIEST_PERSISTED_KEY: &[u8] = b"true_earliest_persisted";


#[derive(Debug, Clone)]
pub struct StateManager {
    db: Arc<DB>,
}

impl StateManager {
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    fn get_u64_opt(&self, key: &[u8]) -> Result<Option<u64>> {
        let cf = self.db.cf_handle(CF_METADATA).ok_or_else(|| anyhow!("Could not get handle for CF: {}", CF_METADATA))?;
        match self.db.get_cf(cf, key)? {
            Some(value) => {
                let bytes: [u8; 8] = value.try_into().map_err(|_| anyhow!("Invalid byte length for u64 metadata key '{:?}'", key))?;
                Ok(Some(u64::from_be_bytes(bytes)))
            }
            None => Ok(None),
        }
    }

    fn set_u64(&self, key: &[u8], value: u64, batch: &mut WriteBatch) -> Result<()> {
        let cf = self.db.cf_handle(CF_METADATA).ok_or_else(|| anyhow!("Could not get handle for CF: {}", CF_METADATA))?;
        batch.put_cf(cf, key, &value.to_be_bytes());
        Ok(())
    }
    
    fn put_u64_direct(&self, key: &[u8], value: u64) -> Result<()> {
        let cf = self.db.cf_handle(CF_METADATA).ok_or_else(|| anyhow!("Could not get handle for CF: {}", CF_METADATA))?;
        self.db.put_cf(cf, key, &value.to_be_bytes())?;
        Ok(())
    }

    pub fn get_latest_persisted(&self) -> Result<Option<u64>> {
        self.get_u64_opt(LATEST_PERSISTED_KEY)
    }

    pub fn get_earliest_hot(&self) -> Result<Option<u64>> {
        self.get_u64_opt(EARLIEST_HOT_KEY)
    }

    pub fn get_highest_contiguous(&self) -> Result<u64> {
        Ok(self.get_u64_opt(HIGHEST_CONTIGUOUS_KEY)?.unwrap_or(0))
    }
    
    pub fn get_true_earliest_persisted(&self) -> Result<Option<u64>> {
        self.get_u64_opt(TRUE_EARLIEST_PERSISTED_KEY)
    }

    pub fn update_true_earliest_if_less(&self, new_earliest: u64) -> Result<()> {
        match self.get_true_earliest_persisted()? {
            Some(current_earliest) => {
                if new_earliest < current_earliest {
                    self.put_u64_direct(TRUE_EARLIEST_PERSISTED_KEY, new_earliest)?;
                }
            }
            None => {
                // If it doesn't exist, this is the first one.
                self.put_u64_direct(TRUE_EARLIEST_PERSISTED_KEY, new_earliest)?;
            }
        }
        Ok(())
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
    
    pub fn set_true_earliest_persisted(&self, block_number: u64, batch: &mut WriteBatch) -> Result<()> {
        self.set_u64(TRUE_EARLIEST_PERSISTED_KEY, block_number, batch)
    }
    
    pub fn initialize_true_earliest_persisted(&self, block_number: u64) -> Result<()> {
        if self.get_true_earliest_persisted()?.is_none() {
            self.put_u64_direct(TRUE_EARLIEST_PERSISTED_KEY, block_number)?;
        }
        Ok(())
    }

    pub fn initialize_earliest_hot(&self, block_number: u64, batch: &mut WriteBatch) -> Result<()> {
        if self.get_earliest_hot()?.is_none() {
            self.set_earliest_hot(block_number, batch)?;
        }
        Ok(())
    }
}
