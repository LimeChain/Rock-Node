use anyhow::{anyhow, Result};
use dashmap::DashMap;
use lazy_static::lazy_static;
use memmap2::Mmap;
use regex::Regex;
use rock_node_core::{config::PersistenceServiceConfig, metrics::MetricsRegistry};
use std::fs::File;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, trace, warn};
use zstd;

lazy_static! {
    // Regex to parse filenames like "blocks-0000000000-0000009999.rbi"
    static ref FILENAME_RE: Regex =
        Regex::new(r"blocks-(\d{10})-(\d{10})\.rbi").unwrap();
}

#[repr(C, packed)]
#[derive(Clone, Copy, Debug)]
struct IndexRecord {
    // These are stored big-endian in the file
    block_number: u64,
    offset: u64,
    length: u32,
}

/// A memory-mapped index file.
#[derive(Debug)]
struct MappedIndex {
    _file: File,
    mmap: Mmap,
    start_block: u64,
    end_block: u64,
    data_path: PathBuf,
}

impl MappedIndex {
    /// Reads an index record for a given block number without allocating.
    fn get_record(&self, block_number: u64) -> Option<IndexRecord> {
        if !(self.start_block..=self.end_block).contains(&block_number) {
            return None;
        }

        let record_size = std::mem::size_of::<IndexRecord>();
        let index = (block_number - self.start_block) as usize;
        let offset = index * record_size;

        if offset + record_size > self.mmap.len() {
            return None;
        }

        let record_bytes = &self.mmap[offset..offset + record_size];
        // SAFETY: We are reading a packed struct from a memory-mapped file slice
        // that is the exact size of the struct. Use read_unaligned to avoid UB.
        let record: IndexRecord =
            unsafe { std::ptr::read_unaligned(record_bytes.as_ptr() as *const IndexRecord) };
        Some(record)
    }
}

#[derive(Debug)]
pub struct ColdReader {
    config: Arc<PersistenceServiceConfig>,
    // The key is the start_block of the chunk for fast lookups.
    indices: Arc<DashMap<u64, MappedIndex>>,
    metrics: Arc<MetricsRegistry>,
}

impl ColdReader {
    pub fn new(config: Arc<PersistenceServiceConfig>, metrics: Arc<MetricsRegistry>) -> Self {
        Self {
            config,
            indices: Arc::new(DashMap::new()),
            metrics,
        }
    }

    /// Parses start/end block numbers from an index file path.
    fn parse_block_range(path: &Path) -> Option<(u64, u64)> {
        let file_name = path.file_name()?.to_str()?;
        let captures = FILENAME_RE.captures(file_name)?;
        let start = captures.get(1)?.as_str().parse().ok()?;
        let end = captures.get(2)?.as_str().parse().ok()?;
        Some((start, end))
    }

    pub fn scan_and_build_index(&self) -> Result<()> {
        trace!(
            "Scanning for cold storage archives in '{}'...",
            self.config.cold_storage_path
        );
        let path = Path::new(&self.config.cold_storage_path);
        if !path.exists() {
            trace!("Cold storage path does not exist, skipping scan.");
            return Ok(());
        }

        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("rbi") {
                if let Err(e) = self.load_index_file(&path) {
                    warn!("Failed to load index file '{:?}': {}. Skipping.", path, e);
                }
            }
        }

        let total_blocks = self.indices.iter().fold(0, |acc, item| {
            acc + (item.value().end_block - item.value().start_block + 1)
        });

        self.metrics
            .persistence_cold_tier_block_count
            .set(total_blocks as i64);
        trace!(
            "Cold storage scan complete. Managing {} block chunks with a total of {} blocks.",
            self.indices.len(),
            total_blocks
        );
        Ok(())
    }

    pub fn load_index_file(&self, index_path: &Path) -> Result<()> {
        let (start_block, end_block) = Self::parse_block_range(index_path)
            .ok_or_else(|| anyhow!("Could not parse block range from {:?}", index_path))?;

        trace!(
            "Memory-mapping new cold storage index file: {:?}",
            index_path
        );
        let data_path = index_path.with_extension("rba");
        if !data_path.exists() {
            return Err(anyhow!("Data file not found for index: {:?}", index_path));
        }

        let file = File::open(index_path)?;
        // SAFETY: The mmap is safe because we require the underlying file to not be changed
        // externally, which is a reasonable assumption for this application's data directory.
        // On crashes, the write-rename-fsync pattern should prevent corruption.
        let mmap = unsafe { Mmap::map(&file)? };

        let mapped_index = MappedIndex {
            _file: file, // Keep file handle alive for the lifetime of the Mmap
            mmap,
            start_block,
            end_block,
            data_path,
        };

        // Update metrics on new load
        let new_count = (end_block - start_block + 1) as i64;
        self.metrics
            .persistence_cold_tier_block_count
            .add(new_count);

        self.indices.insert(start_block, mapped_index);

        Ok(())
    }

    pub fn get_earliest_indexed_block(&self) -> Result<Option<u64>> {
        Ok(self.indices.iter().map(|item| *item.key()).min())
    }

    pub fn read_block(&self, block_number: u64) -> Result<Option<Vec<u8>>> {
        // Find the correct chunk that would contain this block number.
        // We find the greatest `start_block` that is less than or equal to `block_number`.
        let chunk = self
            .indices
            .iter()
            .filter(|item| *item.key() <= block_number)
            .max_by_key(|item| *item.key());

        if let Some(item) = chunk {
            let mapped_index = item.value();
            if let Some(record) = mapped_index.get_record(block_number) {
                // We have the record, now read the data file
                let data_file = File::open(&mapped_index.data_path)?;

                let length = u32::from_be(record.length) as usize;
                let offset = u64::from_be(record.offset);
                let mut compressed_bytes = vec![0; length];

                // Use pread for thread safety, avoiding seeks on a shared file handle
                data_file.read_exact_at(&mut compressed_bytes, offset)?;

                let decompressed = zstd::decode_all(compressed_bytes.as_slice())?;
                return Ok(Some(decompressed));
            }
        }

        debug!(
            "Block #{} not found in any cold storage index.",
            block_number
        );
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cold_storage::writer::ColdWriter;
    use prost::Message;
    use rock_node_protobufs::com::hedera::hapi::block::stream::{block_item, Block};
    use tempfile::TempDir;

    fn make_block(num: u64) -> Block {
        Block {
            items: vec![rock_node_protobufs::com::hedera::hapi::block::stream::BlockItem {
                item: Some(
                    block_item::Item::BlockHeader(
                        rock_node_protobufs::com::hedera::hapi::block::stream::output::BlockHeader {
                            hapi_proto_version: None,
                            software_version: None,
                            number: num,
                            block_timestamp: None,
                            hash_algorithm: 0,
                        },
                    ),
                ),
            }],
        }
    }

    #[test]
    fn scan_and_read_from_cold_storage() {
        let tmp = TempDir::new().unwrap();
        // Use isolated registry for testing to avoid cardinality conflicts
        let registry = prometheus::Registry::new();
        let metrics = Arc::new(MetricsRegistry::with_registry(registry).unwrap());
        let config = Arc::new(PersistenceServiceConfig {
            enabled: true,
            cold_storage_path: tmp.path().to_str().unwrap().to_string(),
            hot_storage_block_count: 10,
            archive_batch_size: 5,
        });

        // Write an archive first
        let writer = ColdWriter::new(config.clone());
        let blocks: Vec<Block> = (200..205).map(make_block).collect();
        let index_path = writer.write_archive(&blocks).unwrap();

        let reader = ColdReader::new(config, metrics);
        reader.scan_and_build_index().unwrap();
        // Should also be able to load explicitly
        reader.load_index_file(&index_path).unwrap();

        let bytes = reader.read_block(203).unwrap().unwrap();
        let decoded = Block::decode(bytes.as_slice()).unwrap();
        match decoded.items.first().unwrap().item.as_ref().unwrap() {
            block_item::Item::BlockHeader(h) => assert_eq!(h.number, 203),
            _ => panic!("unexpected item"),
        }
    }
}
