use anyhow::Result;
use rock_node_core::{events::BlockData, block_reader::BlockReader};
use rocksdb::{DB, Options, WriteBatch};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::info;

#[derive(Serialize, Deserialize, Debug)]
pub struct StoredBlock { pub contents: String }

const LATEST_BLOCK_KEY: &[u8] = b"METADATA::LATEST_PERSISTED_BLOCK";
const EARLIEST_BLOCK_KEY: &[u8] = b"METADATA::EARLIEST_PERSISTED_BLOCK";

#[derive(Clone)]
pub struct StorageManager {
    db: Arc<DB>,
    hot_storage_block_count: u64,
}

impl StorageManager {
    pub fn new(path: &str, hot_storage_block_count: u64) -> Result<Self> {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        let db = Arc::new(DB::open(&opts, path)?);
        Ok(Self { db, hot_storage_block_count })
    }

    pub fn write_block(&self, block_number: u64, data: &BlockData) -> Result<()> {
        let key = block_number.to_be_bytes();
        let stored_block = StoredBlock { contents: data.contents.clone() };
        let value = bincode::serialize(&stored_block)?;

        let mut batch = WriteBatch::default();
        batch.put(&key, &value);                      // FIX: Removed `?`
        batch.put(LATEST_BLOCK_KEY, &key);            // FIX: Removed `?`

        if self.get_earliest_persisted_block_number() == -1 {
            batch.put(EARLIEST_BLOCK_KEY, &key);      // FIX: Removed `?`
        }
        
        // The actual I/O operation that can fail.
        self.db.write(batch)?;

        self.archive_if_needed()
    }

    fn archive_if_needed(&self) -> Result<()> {
        let latest = self.get_latest_persisted_block_number();
        let earliest = self.get_earliest_persisted_block_number();

        if latest <= 0 || earliest < 0 {
            return Ok(());
        }

        let latest_u64 = latest as u64;
        let earliest_u64 = earliest as u64;

        let current_block_count = latest_u64.saturating_sub(earliest_u64) + 1;

        if current_block_count > self.hot_storage_block_count {
            let blocks_to_archive_count = current_block_count - self.hot_storage_block_count;
            let end_block_to_archive = earliest_u64 + blocks_to_archive_count;
            
            info!(
                "Hot storage count ({}) exceeds limit ({}). Archiving {} blocks from #{} to #{}.",
                current_block_count, self.hot_storage_block_count, blocks_to_archive_count, earliest_u64, end_block_to_archive - 1
            );
            
            let mut delete_batch = WriteBatch::default();
            for block_num in earliest_u64..end_block_to_archive {
                delete_batch.delete(block_num.to_be_bytes()); // FIX: Removed `?`
            }
            
            let new_earliest_key = end_block_to_archive.to_be_bytes();
            delete_batch.put(EARLIEST_BLOCK_KEY, &new_earliest_key); // FIX: Removed `?`

            self.db.write(delete_batch)?;

            info!("Archival complete. New earliest block in hot storage is #{}.", end_block_to_archive);
        }
        
        Ok(())
    }
    
    fn read_block_number_from_key(&self, key: &[u8]) -> i64 {
        self.db.get(key)
            .unwrap_or(None)
            .and_then(|val| val.try_into().ok())
            .map(u64::from_be_bytes)
            .map(|n| n as i64)
            .unwrap_or(-1)
    }
}

impl BlockReader for StorageManager {
    fn get_latest_persisted_block_number(&self) -> i64 {
        self.read_block_number_from_key(LATEST_BLOCK_KEY)
    }

    fn get_earliest_persisted_block_number(&self) -> i64 {
        self.read_block_number_from_key(EARLIEST_BLOCK_KEY)
    }
}
