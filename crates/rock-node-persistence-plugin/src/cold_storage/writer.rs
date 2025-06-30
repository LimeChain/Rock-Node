use anyhow::{anyhow, Result};
use prost::Message;
use rock_node_core::config::PersistenceServiceConfig;
use rock_node_protobufs::com::hedera::hapi::block::stream::{block_item, Block};
use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use zstd;

/// A record in the .rbi (Rock Block Index) file.
#[repr(C, packed)]
struct IndexRecord {
    block_number: u64,
    offset: u64,
    length: u32,
}

/// Responsible for writing blocks to durable, indexed archive files.
#[derive(Debug, Clone)]
pub struct ColdWriter {
    config: Arc<PersistenceServiceConfig>,
}

impl ColdWriter {
    pub fn new(config: Arc<PersistenceServiceConfig>) -> Self {
        Self { config }
    }

    pub fn write_archive(&self, blocks: &[Block]) -> Result<PathBuf> {
        if blocks.is_empty() {
            // This case should ideally not be hit, but returning an error is safe.
            return Err(anyhow!("Cannot write an archive for an empty block slice."));
        }

        let first_num = get_block_number(blocks.first().unwrap())?;
        let last_num = get_block_number(blocks.last().unwrap())?;

        let base_path = Path::new(&self.config.cold_storage_path);
        std::fs::create_dir_all(base_path)?;

        let filename_base = format!("blocks-{:010}-{:010}", first_num, last_num);
        let data_filename = format!("{}.rba", filename_base);
        let index_filename = format!("{}.rbi", filename_base);

        let temp_data_path = base_path.join(format!("{}.tmp", data_filename));
        let temp_index_path = base_path.join(format!("{}.tmp", index_filename));

        self.write_temp_files(blocks, &temp_data_path, &temp_index_path)?;

        let final_data_path = base_path.join(data_filename);
        let final_index_path = base_path.join(index_filename);
        std::fs::rename(&temp_data_path, final_data_path)?;
        std::fs::rename(&temp_index_path, &final_index_path)?;

        Ok(final_index_path)
    }

    fn write_temp_files(
        &self,
        blocks: &[Block],
        temp_data_path: &Path,
        temp_index_path: &Path,
    ) -> Result<()> {
        let data_file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(temp_data_path)?;
        let index_file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(temp_index_path)?;

        let mut data_writer = BufWriter::new(data_file);
        let mut index_writer = BufWriter::new(index_file);
        let mut current_offset: u64 = 0;

        for block in blocks {
            let block_number = get_block_number(block)?;
            let compressed_bytes = zstd::encode_all(block.encode_to_vec().as_slice(), 0)?;
            let length = compressed_bytes.len() as u32;

            data_writer.write_all(&compressed_bytes)?;

            let record = IndexRecord {
                block_number: block_number.to_be(),
                offset: current_offset.to_be(),
                length: length.to_be(),
            };

            let record_bytes: &[u8] = unsafe {
                std::slice::from_raw_parts(
                    &record as *const _ as *const u8,
                    std::mem::size_of::<IndexRecord>(),
                )
            };
            index_writer.write_all(record_bytes)?;

            current_offset += length as u64;
        }

        data_writer.flush()?;
        index_writer.flush()?;
        data_writer.into_inner()?.sync_all()?;
        index_writer.into_inner()?.sync_all()?;

        Ok(())
    }
}

fn get_block_number(block: &Block) -> Result<u64> {
    if let Some(first_item) = block.items.first() {
        if let Some(block_item::Item::BlockHeader(header)) = &first_item.item {
            return Ok(header.number);
        }
    }
    Err(anyhow!(
        "Block is malformed or first item is not a BlockHeader"
    ))
}
