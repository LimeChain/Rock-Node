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

#[cfg(test)]
mod tests {
    use super::*;
    use rock_node_core::database::{DatabaseManager, CF_GAPS};
    use tempfile::TempDir;

    fn setup_db() -> (TempDir, Arc<DB>) {
        let temp_dir = TempDir::new().unwrap();
        let manager = DatabaseManager::new(temp_dir.path().to_str().unwrap()).unwrap();
        (temp_dir, manager.db_handle())
    }

    #[test]
    fn initializes_and_reads_u64_metadata_keys() {
        let (_tmp, db) = setup_db();
        let state = StateManager::new(db.clone());
        assert_eq!(state.get_latest_persisted().unwrap(), None);

        let mut batch = WriteBatch::default();
        state.set_latest_persisted(42, &mut batch).unwrap();
        state.set_highest_contiguous(40, &mut batch).unwrap();
        state.set_true_earliest_persisted(10, &mut batch).unwrap();
        state.set_earliest_hot(11, &mut batch).unwrap();
        db.write(batch).unwrap();

        assert_eq!(state.get_latest_persisted().unwrap(), Some(42));
        assert_eq!(state.get_highest_contiguous().unwrap(), 40);
        assert_eq!(state.get_true_earliest_persisted().unwrap(), Some(10));
        assert_eq!(state.get_earliest_hot().unwrap(), Some(11));
    }

    #[test]
    fn initialize_helpers_only_set_when_missing() {
        let (_tmp, db) = setup_db();
        let state = StateManager::new(db.clone());

        // highest_contiguous defaults to 0
        assert_eq!(state.get_highest_contiguous().unwrap(), 0);
        state.initialize_highest_contiguous(5).unwrap();
        assert_eq!(state.get_highest_contiguous().unwrap(), 5);
        // Calling again should not overwrite
        state.initialize_highest_contiguous(7).unwrap();
        assert_eq!(state.get_highest_contiguous().unwrap(), 5);

        // true_earliest_persisted missing -> set
        assert_eq!(state.get_true_earliest_persisted().unwrap(), None);
        state.initialize_true_earliest_persisted(12).unwrap();
        assert_eq!(state.get_true_earliest_persisted().unwrap(), Some(12));
        // Calling again should not overwrite
        state.initialize_true_earliest_persisted(9).unwrap();
        assert_eq!(state.get_true_earliest_persisted().unwrap(), Some(12));

        // earliest_hot set via initialize_earliest_hot only if missing
        let mut batch = WriteBatch::default();
        state.initialize_earliest_hot(20, &mut batch).unwrap();
        db.write(batch).unwrap();
        assert_eq!(state.get_earliest_hot().unwrap(), Some(20));
        let mut batch2 = WriteBatch::default();
        state.initialize_earliest_hot(25, &mut batch2).unwrap();
        db.write(batch2).unwrap();
        assert_eq!(state.get_earliest_hot().unwrap(), Some(20));
    }

    #[test]
    fn gap_range_merge_and_fill_logic() {
        let (_tmp, db) = setup_db();
        let state = StateManager::new(db.clone());
        let cf_gaps = db.cf_handle(CF_GAPS).unwrap();

        // Add [5,10]
        let mut batch = WriteBatch::default();
        state.add_gap_range(5, 10, &mut batch).unwrap();
        db.write(batch).unwrap();

        // Add adjacent [11,12] -> should merge to [5,12]
        let mut batch = WriteBatch::default();
        state.add_gap_range(11, 12, &mut batch).unwrap();
        db.write(batch).unwrap();

        // Verify single merged range [5,12]
        let mut iter = db.iterator_cf(cf_gaps, IteratorMode::Start);
        let (k, v) = iter.next().unwrap().unwrap();
        assert_eq!(u64::from_be_bytes(k.as_ref().try_into().unwrap()), 5);
        assert_eq!(u64::from_be_bytes(v.as_ref().try_into().unwrap()), 12);
        assert!(iter.next().is_none());

        // Fill block 5 -> should become [6,12]
        let mut batch = WriteBatch::default();
        state.fill_gap_block(5, &mut batch).unwrap();
        db.write(batch).unwrap();
        let (k, v) = db
            .iterator_cf(cf_gaps, IteratorMode::Start)
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(u64::from_be_bytes(k.as_ref().try_into().unwrap()), 6);
        assert_eq!(u64::from_be_bytes(v.as_ref().try_into().unwrap()), 12);

        // Fill middle block 8 -> becomes [6,7] and [9,12]
        let mut batch = WriteBatch::default();
        state.fill_gap_block(8, &mut batch).unwrap();
        db.write(batch).unwrap();
        let mut iter = db.iterator_cf(cf_gaps, IteratorMode::Start);
        let (k1, v1) = iter.next().unwrap().unwrap();
        let (k2, v2) = iter.next().unwrap().unwrap();
        assert_eq!(u64::from_be_bytes(k1.as_ref().try_into().unwrap()), 6);
        assert_eq!(u64::from_be_bytes(v1.as_ref().try_into().unwrap()), 7);
        assert_eq!(u64::from_be_bytes(k2.as_ref().try_into().unwrap()), 9);
        assert_eq!(u64::from_be_bytes(v2.as_ref().try_into().unwrap()), 12);

        // Add overlapping [7,9] -> merge to [6,12]
        let mut batch = WriteBatch::default();
        state.add_gap_range(7, 9, &mut batch).unwrap();
        db.write(batch).unwrap();
        let (k, v) = db
            .iterator_cf(cf_gaps, IteratorMode::Start)
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(u64::from_be_bytes(k.as_ref().try_into().unwrap()), 6);
        assert_eq!(u64::from_be_bytes(v.as_ref().try_into().unwrap()), 12);
    }
}
