use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use dashmap::DashMap;
use uuid::Uuid;

/// The state of a single publisher session in the block race.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// The session has just connected and is competing to be primary.
    New,
    /// This session "won" the race and is sending the block data.
    Primary,
    /// This session "lost" the race and has been told to skip this block.
    Behind,
}

/// The central, shared state for the entire Publish Plugin.
/// It's wrapped in an Arc to be shared safely across all session tasks.
#[derive(Debug)]
pub struct SharedState {
    /// Maps a block number to the ID of the session that "won" the race for it.
    pub block_winners: DashMap<u64, Uuid>,
    /// The latest block number that has been confirmed as persisted by the system.
    /// We use an AtomicU64 for safe concurrent reads from many session tasks.
    pub latest_persisted_block: AtomicI64,
}

impl SharedState {
    pub fn new() -> Self {
        Self {
            block_winners: DashMap::new(),
            // u64::MAX is used to represent -1, for a clean state before genesis block 0.
            latest_persisted_block: AtomicI64::new(-1),
        }
    }

    pub fn get_latest_persisted_block(&self) -> i64 {
        self.latest_persisted_block.load(Ordering::Relaxed)
    }

    pub fn set_latest_persisted_block(&self, block_number: i64) {
        self.latest_persisted_block.store(block_number, Ordering::Relaxed);
    }
}
