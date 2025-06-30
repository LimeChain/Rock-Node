use anyhow::Result;
use dashmap::DashMap;
use rock_node_core::config::PersistenceServiceConfig;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;
use zstd;

// Must match the writer's IndexRecord
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct IndexRecord {
    block_number: u64,
    offset: u64,
    length: u32,
}

#[derive(Debug, Clone)]
struct ArchiveLocation {
    archive_path: PathBuf,
    offset: u64,
    length: u32,
}

/// Scans and reads from cold storage archives.
#[derive(Debug, Clone)]
pub struct ColdReader {
    config: Arc<PersistenceServiceConfig>,
    index: Arc<DashMap<u64, ArchiveLocation>>,
}

impl ColdReader {
    pub fn new(config: Arc<PersistenceServiceConfig>) -> Self {
        Self {
            config,
            index: Arc::new(DashMap::new()),
        }
    }

    pub fn scan_and_build_index(&self) -> Result<()> {
        info!(
            "Scanning for cold storage archives in '{}'...",
            self.config.cold_storage_path
        );
        let path = Path::new(&self.config.cold_storage_path);
        if !path.exists() {
            info!("Cold storage path does not exist, skipping scan.");
            return Ok(());
        }

        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("rbi") {
                self.load_index_file(&path)?;
            }
        }
        info!(
            "Cold storage scan complete. Indexed {} blocks.",
            self.index.len()
        );
        Ok(())
    }

    fn load_index_file(&self, index_path: &Path) -> Result<()> {
        let mut file = File::open(index_path)?;
        let mut buffer = vec![0; std::mem::size_of::<IndexRecord>()];
        let data_path = index_path.with_extension("rba");

        while file.read_exact(&mut buffer).is_ok() {
            let record: IndexRecord = unsafe { std::ptr::read(buffer.as_ptr() as *const _) };
            let location = ArchiveLocation {
                archive_path: data_path.clone(),
                offset: u64::from_be(record.offset),
                length: u32::from_be(record.length),
            };
            self.index
                .insert(u64::from_be(record.block_number), location);
        }
        Ok(())
    }
    
    // FIX: New method to get the earliest block number from the in-memory index.
    pub fn get_earliest_indexed_block(&self) -> Result<Option<u64>> {
        Ok(self.index.iter().map(|item| *item.key()).min())
    }

    pub fn read_block(&self, block_number: u64) -> Result<Option<Vec<u8>>> {
        if let Some(location) = self.index.get(&block_number) {
            let mut file = File::open(&location.archive_path)?;
            file.seek(SeekFrom::Start(location.offset))?;

            let mut compressed_bytes = vec![0; location.length as usize];
            file.read_exact(&mut compressed_bytes)?;

            let decompressed = zstd::decode_all(compressed_bytes.as_slice())?;
            return Ok(Some(decompressed));
        }
        Ok(None)
    }
}
