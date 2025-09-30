use anyhow::Result;
use rocksdb::{ColumnFamilyDescriptor, Options, DB};
use std::sync::Arc;

// Define the names of our Column Families as public constants
// so they can be referenced safely from other crates.
pub const CF_METADATA: &str = "metadata";
pub const CF_HOT_BLOCKS: &str = "hot_blocks";
pub const CF_STATE_DATA: &str = "state_data";
pub const CF_GAPS: &str = "gaps";
pub const CF_SKIPPED_BATCHES: &str = "skipped_batches";

// A constant for the State Plugin's metadata key to track the last processed block.
pub const STATE_LAST_PROCESSED_BLOCK: &[u8] = b"state_last_processed_block";

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
            ColumnFamilyDescriptor::new(CF_GAPS, Options::default()),
            ColumnFamilyDescriptor::new(CF_SKIPPED_BATCHES, Options::default()),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread;
    use tempfile::TempDir;

    #[test]
    fn test_database_manager_new_success() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().to_str().unwrap();

        let db_manager = DatabaseManager::new(db_path);
        assert!(db_manager.is_ok());
    }

    #[test]
    fn test_database_manager_creates_required_column_families() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().to_str().unwrap();

        let db_manager = DatabaseManager::new(db_path).unwrap();
        let db = db_manager.db_handle();

        // Test that all required column families exist
        assert!(db.cf_handle("default").is_some());
        assert!(db.cf_handle(CF_METADATA).is_some());
        assert!(db.cf_handle(CF_HOT_BLOCKS).is_some());
        assert!(db.cf_handle(CF_STATE_DATA).is_some());
        assert!(db.cf_handle(CF_GAPS).is_some());
        assert!(db.cf_handle(CF_SKIPPED_BATCHES).is_some());
    }

    #[test]
    fn test_database_manager_invalid_path() {
        // Test with an invalid path (using a file that already exists)
        let temp_file = tempfile::NamedTempFile::new().unwrap();
        let invalid_path = temp_file.path().to_str().unwrap();

        let result = DatabaseManager::new(invalid_path);
        assert!(result.is_err());
    }

    #[test]
    fn test_database_manager_concurrent_access() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().to_str().unwrap();

        let db_manager = Arc::new(DatabaseManager::new(db_path).unwrap());
        let counter = Arc::new(AtomicUsize::new(0));

        let handles: Vec<_> = (0..10)
            .map(|i| {
                let db_manager = Arc::clone(&db_manager);
                let counter = Arc::clone(&counter);
                thread::spawn(move || {
                    let db = db_manager.db_handle();
                    let key = format!("test_key_{}", i);
                    let value = format!("test_value_{}", i);

                    // Test write
                    db.put(key.as_bytes(), value.as_bytes()).unwrap();

                    // Test read
                    let retrieved = db.get(key.as_bytes()).unwrap();
                    assert_eq!(retrieved, Some(value.into_bytes()));

                    counter.fetch_add(1, Ordering::SeqCst);
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(counter.load(Ordering::SeqCst), 10);
    }

    #[test]
    fn test_database_manager_reopen_existing() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().to_str().unwrap();

        // Create database and write some data
        {
            let db_manager = DatabaseManager::new(db_path).unwrap();
            let db = db_manager.db_handle();
            db.put(b"test_key", b"test_value").unwrap();
        }

        // Reopen database and verify data is preserved
        {
            let db_manager = DatabaseManager::new(db_path).unwrap();
            let db = db_manager.db_handle();
            let value = db.get(b"test_key").unwrap();
            assert_eq!(value, Some(b"test_value".to_vec()));
        }
    }

    #[test]
    fn test_database_manager_column_family_operations() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().to_str().unwrap();

        let db_manager = DatabaseManager::new(db_path).unwrap();
        let db = db_manager.db_handle();

        // Test operations on different column families
        let cf_metadata = db.cf_handle(CF_METADATA).unwrap();
        let cf_hot_blocks = db.cf_handle(CF_HOT_BLOCKS).unwrap();

        // Write to different CFs
        db.put_cf(cf_metadata, b"meta_key", b"meta_value").unwrap();
        db.put_cf(cf_hot_blocks, b"block_key", b"block_value")
            .unwrap();

        // Read from different CFs
        let meta_value = db.get_cf(cf_metadata, b"meta_key").unwrap();
        let block_value = db.get_cf(cf_hot_blocks, b"block_key").unwrap();

        assert_eq!(meta_value, Some(b"meta_value".to_vec()));
        assert_eq!(block_value, Some(b"block_value".to_vec()));

        // Verify isolation between CFs
        let non_existent = db.get_cf(cf_metadata, b"block_key").unwrap();
        assert_eq!(non_existent, None);
    }

    #[test]
    fn test_database_manager_handle_sharing() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().to_str().unwrap();

        let db_manager = DatabaseManager::new(db_path).unwrap();

        // Get multiple handles
        let handle1 = db_manager.db_handle();
        let handle2 = db_manager.db_handle();

        // Verify they point to the same underlying database
        handle1.put(b"shared_key", b"shared_value").unwrap();
        let value = handle2.get(b"shared_key").unwrap();
        assert_eq!(value, Some(b"shared_value".to_vec()));
    }

    #[test]
    fn test_database_constants() {
        // Test that our constants are properly defined
        assert_eq!(CF_METADATA, "metadata");
        assert_eq!(CF_HOT_BLOCKS, "hot_blocks");
        assert_eq!(CF_STATE_DATA, "state_data");
        assert_eq!(CF_GAPS, "gaps");
        assert_eq!(CF_SKIPPED_BATCHES, "skipped_batches");
        assert_eq!(STATE_LAST_PROCESSED_BLOCK, b"state_last_processed_block");
    }

    #[test]
    fn test_database_manager_debug_formatting() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().to_str().unwrap();

        let db_manager = DatabaseManager::new(db_path).unwrap();
        let debug_str = format!("{:?}", db_manager);
        assert!(debug_str.contains("DatabaseManager"));
    }
}
