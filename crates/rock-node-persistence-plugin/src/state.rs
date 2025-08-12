use anyhow::{anyhow, Result};
use rock_node_core::database::{CF_GAPS, CF_METADATA, CF_SKIPPED_BATCHES};
use rocksdb::{IteratorMode, WriteBatch, DB};
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
        let cf = self
            .db
            .cf_handle(CF_METADATA)
            .ok_or_else(|| anyhow!("Could not get handle for CF: {}", CF_METADATA))?;
        match self.db.get_cf(cf, key)? {
            Some(value) => {
                let bytes: [u8; 8] = value
                    .try_into()
                    .map_err(|_| anyhow!("Invalid byte length for u64 metadata key '{:?}'", key))?;
                Ok(Some(u64::from_be_bytes(bytes)))
            }
            None => Ok(None),
        }
    }

    fn set_u64(&self, key: &[u8], value: u64, batch: &mut WriteBatch) -> Result<()> {
        let cf = self
            .db
            .cf_handle(CF_METADATA)
            .ok_or_else(|| anyhow!("Could not get handle for CF: {}", CF_METADATA))?;
        batch.put_cf(cf, key, &value.to_be_bytes());
        Ok(())
    }

    fn put_u64_direct(&self, key: &[u8], value: u64) -> Result<()> {
        let cf = self
            .db
            .cf_handle(CF_METADATA)
            .ok_or_else(|| anyhow!("Could not get handle for CF: {}", CF_METADATA))?;
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

    pub fn set_true_earliest_persisted(
        &self,
        block_number: u64,
        batch: &mut WriteBatch,
    ) -> Result<()> {
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

    /// Initializes `highest_contiguous` to the provided value only if it is not already set.
    pub fn initialize_highest_contiguous(&self, value: u64) -> Result<()> {
        if self.get_u64_opt(HIGHEST_CONTIGUOUS_KEY)?.is_none() {
            self.put_u64_direct(HIGHEST_CONTIGUOUS_KEY, value)?;
        }
        Ok(())
    }

    pub fn find_containing_gap(&self, block_number: u64) -> Result<Option<(u64, u64)>> {
        let cf = self
            .db
            .cf_handle(CF_GAPS)
            .ok_or_else(|| anyhow!("Could not get handle for CF: {}", CF_GAPS))?;
        let mut iter = self.db.iterator_cf(
            cf,
            IteratorMode::From(&block_number.to_be_bytes(), rocksdb::Direction::Reverse),
        );
        if let Some(Ok((key_bytes, val_bytes))) = iter.next() {
            let start = u64::from_be_bytes(key_bytes.as_ref().try_into()?);
            let end = u64::from_be_bytes(val_bytes.as_ref().try_into()?);
            if block_number >= start && block_number <= end {
                return Ok(Some((start, end)));
            }
        }
        Ok(None)
    }

    pub fn add_gap_range(&self, start: u64, end: u64, batch: &mut WriteBatch) -> Result<()> {
        let cf = self
            .db
            .cf_handle(CF_GAPS)
            .ok_or_else(|| anyhow!("Could not get handle for CF: {}", CF_GAPS))?;

        if end < start {
            return Ok(()); // ignore invalid input defensively
        }

        // Determine the leftmost overlapping/adjacent gap (if any) and expand bounds.
        let mut final_start = start;
        let mut final_end = end;

        {
            let mut rev_iter = self.db.iterator_cf(
                cf,
                IteratorMode::From(&start.to_be_bytes(), rocksdb::Direction::Reverse),
            );
            if let Some(Ok((k_bytes, v_bytes))) = rev_iter.next() {
                let k_start = u64::from_be_bytes(k_bytes.as_ref().try_into()?);
                let v_end = u64::from_be_bytes(v_bytes.as_ref().try_into()?);
                // Overlap or adjacency to the left side
                if v_end.saturating_add(1) >= start {
                    if k_start < final_start {
                        final_start = k_start;
                    }
                    if v_end > final_end {
                        final_end = v_end;
                    }
                }
            }
        }

        // Sweep forward from final_start and collapse all overlapping/adjacent gaps into one.
        {
            let mut fwd_iter = self.db.iterator_cf(
                cf,
                IteratorMode::From(&final_start.to_be_bytes(), rocksdb::Direction::Forward),
            );
            while let Some(Ok((k_bytes, v_bytes))) = fwd_iter.next() {
                let k_start = u64::from_be_bytes(k_bytes.as_ref().try_into()?);
                if k_start > final_end.saturating_add(1) {
                    break;
                }
                let v_end = u64::from_be_bytes(v_bytes.as_ref().try_into()?);
                if v_end > final_end {
                    final_end = v_end;
                }
                // Remove this existing range; we'll write the merged range at the end.
                batch.delete_cf(cf, &k_bytes);
            }
        }

        batch.put_cf(cf, &final_start.to_be_bytes(), &final_end.to_be_bytes());
        Ok(())
    }

    pub fn fill_gap_block(&self, block_number: u64, batch: &mut WriteBatch) -> Result<()> {
        let cf = self
            .db
            .cf_handle(CF_GAPS)
            .ok_or_else(|| anyhow!("Could not get handle for CF: {}", CF_GAPS))?;
        if let Some((start, end)) = self.find_containing_gap(block_number)? {
            batch.delete_cf(cf, &start.to_be_bytes());
            if block_number > start {
                batch.put_cf(cf, &start.to_be_bytes(), &(block_number - 1).to_be_bytes());
            }
            if block_number < end {
                batch.put_cf(cf, &(block_number + 1).to_be_bytes(), &end.to_be_bytes());
            }
        }
        Ok(())
    }

    pub fn add_skipped_batch(&self, batch_start_block: u64, batch: &mut WriteBatch) -> Result<()> {
        let cf = self
            .db
            .cf_handle(CF_SKIPPED_BATCHES)
            .ok_or_else(|| anyhow!("Could not get handle for CF: {}", CF_SKIPPED_BATCHES))?;
        batch.put_cf(cf, &batch_start_block.to_be_bytes(), b"");
        Ok(())
    }

    pub fn remove_skipped_batch(
        &self,
        batch_start_block: u64,
        batch: &mut WriteBatch,
    ) -> Result<()> {
        let cf = self
            .db
            .cf_handle(CF_SKIPPED_BATCHES)
            .ok_or_else(|| anyhow!("Could not get handle for CF: {}", CF_SKIPPED_BATCHES))?;
        batch.delete_cf(cf, &batch_start_block.to_be_bytes());
        Ok(())
    }

    pub fn is_batch_skipped(&self, batch_start_block: u64) -> Result<bool> {
        let cf = self
            .db
            .cf_handle(CF_SKIPPED_BATCHES)
            .ok_or_else(|| anyhow!("Could not get handle for CF: {}", CF_SKIPPED_BATCHES))?;
        Ok(self
            .db
            .get_cf(cf, &batch_start_block.to_be_bytes())?
            .is_some())
    }
}
