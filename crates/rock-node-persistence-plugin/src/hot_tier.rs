use anyhow::{anyhow, Result};
use prost::Message;
use rock_node_core::database::CF_HOT_BLOCKS;
use rock_node_protobufs::com::hedera::hapi::block::stream::Block;
use rocksdb::{IteratorMode, WriteBatch, DB};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Serialize, Deserialize, Debug)]
pub struct StoredBlock {
    pub contents: Vec<u8>,
}

/// Manages all block data within the 'hot_blocks' Column Family.
#[derive(Debug, Clone)]
pub struct HotTier {
    db: Arc<DB>,
}

impl HotTier {
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    pub fn read_block(&self, block_number: u64) -> Result<Option<Vec<u8>>> {
        let cf = self
            .db
            .cf_handle(CF_HOT_BLOCKS)
            .ok_or_else(|| anyhow!("Could not get handle for CF: {}", CF_HOT_BLOCKS))?;

        let key = block_number.to_be_bytes();
        match self.db.get_cf(cf, key)? {
            Some(db_vec) => {
                let stored_block: StoredBlock = bincode::deserialize(&db_vec)?;
                Ok(Some(stored_block.contents))
            },
            None => Ok(None),
        }
    }

    pub fn get_earliest_block_number(&self) -> Result<Option<u64>> {
        let cf = self
            .db
            .cf_handle(CF_HOT_BLOCKS)
            .ok_or_else(|| anyhow!("Could not get handle for CF: {}", CF_HOT_BLOCKS))?;

        let mut iter = self.db.iterator_cf(cf, IteratorMode::Start);

        if let Some(Ok((key_bytes, _))) = iter.next() {
            let key_array: [u8; 8] = key_bytes
                .as_ref()
                .try_into()
                .map_err(|_| anyhow!("Invalid key length in hot_blocks CF"))?;
            Ok(Some(u64::from_be_bytes(key_array)))
        } else {
            Ok(None)
        }
    }

    /// Checks if a full batch of blocks exists in the hot tier without any gaps.
    pub fn is_batch_complete(&self, start_block: u64, batch_size: u64) -> Result<bool> {
        let cf = self
            .db
            .cf_handle(CF_HOT_BLOCKS)
            .ok_or_else(|| anyhow!("Could not get handle for CF: {}", CF_HOT_BLOCKS))?;
        for i in 0..batch_size {
            if self
                .db
                .get_cf(cf, (start_block + i).to_be_bytes())?
                .is_none()
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub fn read_block_batch(&self, start_block: u64, count: u64) -> Result<Vec<Block>> {
        let mut blocks = Vec::with_capacity(count as usize);
        for i in 0..count {
            let block_number = start_block + i;
            if let Some(block_bytes) = self.read_block(block_number)? {
                blocks.push(Block::decode(block_bytes.as_slice())?);
            } else {
                return Err(anyhow!(
                    "Missing block #{} in hot tier during batch read for archival",
                    block_number
                ));
            }
        }
        Ok(blocks)
    }

    pub fn add_block_to_batch(
        &self,
        block: &Block,
        block_number: u64,
        batch: &mut WriteBatch,
    ) -> Result<()> {
        let cf = self
            .db
            .cf_handle(CF_HOT_BLOCKS)
            .ok_or_else(|| anyhow!("Could not get handle for CF: {}", CF_HOT_BLOCKS))?;

        let key = block_number.to_be_bytes();
        let mut block_bytes = Vec::new();
        block.encode(&mut block_bytes)?;

        let stored_block = StoredBlock {
            contents: block_bytes,
        };
        let value = bincode::serialize(&stored_block)?;

        batch.put_cf(cf, key, &value);
        Ok(())
    }

    pub fn add_delete_to_batch(&self, block_number: u64, batch: &mut WriteBatch) -> Result<()> {
        let cf = self
            .db
            .cf_handle(CF_HOT_BLOCKS)
            .ok_or_else(|| anyhow!("Could not get handle for CF: {}", CF_HOT_BLOCKS))?;

        let key = block_number.to_be_bytes();
        batch.delete_cf(cf, key);
        Ok(())
    }

    pub fn commit_batch(&self, batch: WriteBatch) -> Result<()> {
        self.db.write(batch)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rock_node_core::database::DatabaseManager;
    use rock_node_protobufs::com::hedera::hapi::block::stream::block_item::Item;
    use tempfile::TempDir;

    fn setup_hot() -> (TempDir, HotTier, Arc<DB>) {
        let tmp = TempDir::new().unwrap();
        let manager = DatabaseManager::new(tmp.path().to_str().unwrap()).unwrap();
        let db = manager.db_handle();
        (tmp, HotTier::new(db.clone()), db)
    }

    #[test]
    fn read_and_write_single_block() {
        let (_tmp, hot, _db) = setup_hot();
        // build a minimal block with header
        let block = rock_node_protobufs::com::hedera::hapi::block::stream::Block {
            items: vec![rock_node_protobufs::com::hedera::hapi::block::stream::BlockItem {
                item: Some(
                    rock_node_protobufs::com::hedera::hapi::block::stream::block_item::Item::BlockHeader(
                        rock_node_protobufs::com::hedera::hapi::block::stream::output::BlockHeader {
                            hapi_proto_version: None,
                            software_version: None,
                            number: 123,
                            block_timestamp: None,
                            hash_algorithm: 0,
                        },
                    ),
                ),
            }],
        };

        let mut batch = WriteBatch::default();
        hot.add_block_to_batch(&block, 123, &mut batch).unwrap();
        hot.commit_batch(batch).unwrap();

        let bytes = hot.read_block(123).unwrap().unwrap();
        let decoded = Block::decode(bytes.as_slice()).unwrap();
        match decoded.items.first().unwrap().item.as_ref().unwrap() {
            Item::BlockHeader(h) => assert_eq!(h.number, 123),
            _ => panic!("unexpected item"),
        }
        assert_eq!(hot.get_earliest_block_number().unwrap(), Some(123));

        // delete path
        let mut batch = WriteBatch::default();
        hot.add_delete_to_batch(123, &mut batch).unwrap();
        hot.commit_batch(batch).unwrap();
        assert!(hot.read_block(123).unwrap().is_none());
    }

    #[test]
    fn batch_complete_and_batch_read() {
        let (_tmp, hot, _db) = setup_hot();
        // insert 5 sequential blocks starting at 100
        let mut write_batch = WriteBatch::default();
        for i in 0..5u64 {
            let num = 100 + i;
            let block = rock_node_protobufs::com::hedera::hapi::block::stream::Block {
                items: vec![rock_node_protobufs::com::hedera::hapi::block::stream::BlockItem {
                    item: Some(
                        rock_node_protobufs::com::hedera::hapi::block::stream::block_item::Item::BlockHeader(
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
            };
            hot.add_block_to_batch(&block, num, &mut write_batch)
                .unwrap();
        }
        hot.commit_batch(write_batch).unwrap();

        assert!(hot.is_batch_complete(100, 5).unwrap());
        let blocks = hot.read_block_batch(100, 5).unwrap();
        assert_eq!(blocks.len(), 5);
        assert_eq!(blocks.first().unwrap().items.len() > 0, true);
    }
}
