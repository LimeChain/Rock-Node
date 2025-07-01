use anyhow::Result;
use rocksdb::{ColumnFamilyDescriptor, Options, DB};
use std::sync::Arc;

// Define the names of our Column Families as public constants
// so they can be referenced safely from other crates.
pub const CF_METADATA: &str = "metadata";
pub const CF_HOT_BLOCKS: &str = "hot_blocks";
pub const CF_STATE_DATA: &str = "state_data"; // For the future State Plugin

/// Manages the single RocksDB instance and provides access to its Column Families.
#[derive(Debug)]
pub struct DatabaseManager {
    // The main DB instance is wrapped in an Arc for safe, shared access.
    db: Arc<DB>,
}

impl DatabaseManager {
    /// Opens the database with a predefined set of Column Families.
    pub fn new(path: &str) -> Result<Self> {
        let cf_descriptors = vec![
            // The "default" column family is always required.
            ColumnFamilyDescriptor::new("default", Options::default()),
            ColumnFamilyDescriptor::new(CF_METADATA, Options::default()),
            ColumnFamilyDescriptor::new(CF_HOT_BLOCKS, Options::default()),
            ColumnFamilyDescriptor::new(CF_STATE_DATA, Options::default()),
        ];

        let mut db_opts = Options::default();
        db_opts.create_if_missing(true);
        db_opts.create_missing_column_families(true);

        let db = DB::open_cf_descriptors(&db_opts, path, cf_descriptors)?;

        Ok(Self { db: Arc::new(db) })
    }

    /// Provides shared access to the main DB object.
    pub fn db_handle(&self) -> Arc<DB> {
        self.db.clone()
    }
}
