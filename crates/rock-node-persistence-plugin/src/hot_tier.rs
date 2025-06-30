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
        match self.db.get_cf(cf, &key)? {
            Some(db_vec) => {
                let stored_block: StoredBlock = bincode::deserialize(&db_vec)?;
                Ok(Some(stored_block.contents))
            }
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

    pub fn read_block_batch(&self, start_block: u64, count: u64) -> Result<Vec<Block>> {
        let mut blocks = Vec::with_capacity(count as usize);
        for i in 0..count {
            let block_number = start_block + i;
            if let Some(block_bytes) = self.read_block(block_number)? {
                blocks.push(Block::decode(block_bytes.as_slice())?);
            } else {
                return Err(anyhow!(
                    "Missing block #{} in hot tier during batch read",
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

        batch.put_cf(cf, &key, &value);
        Ok(())
    }

    pub fn add_delete_to_batch(&self, block_number: u64, batch: &mut WriteBatch) -> Result<()> {
        let cf = self
            .db
            .cf_handle(CF_HOT_BLOCKS)
            .ok_or_else(|| anyhow!("Could not get handle for CF: {}", CF_HOT_BLOCKS))?;

        let key = block_number.to_be_bytes();
        batch.delete_cf(cf, &key);
        Ok(())
    }

    pub fn commit_batch(&self, batch: WriteBatch) -> Result<()> {
        self.db.write(batch)?;
        Ok(())
    }
}
