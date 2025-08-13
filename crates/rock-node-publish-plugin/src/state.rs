use dashmap::DashMap;
use rock_node_protobufs::org::hiero::block::api::PublishStreamResponse;
use std::sync::atomic::{AtomicI64, Ordering};
use tokio::sync::mpsc;
use tonic::Status;
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
#[derive(Debug)]
pub struct SharedState {
    /// Maps a block number to the ID of the session that "won" the race for it.
    pub block_winners: DashMap<u64, Uuid>,
    /// The latest block number that has been confirmed as persisted by the system.
    pub latest_persisted_block: AtomicI64,
    /// Maps a session ID to its response channel sender to broadcast messages.
    pub active_sessions: DashMap<Uuid, mpsc::Sender<Result<PublishStreamResponse, Status>>>,
}

impl SharedState {
    pub fn new() -> Self {
        Self {
            block_winners: DashMap::new(),
            latest_persisted_block: AtomicI64::new(-1),
            active_sessions: DashMap::new(),
        }
    }

    pub fn get_latest_persisted_block(&self) -> i64 {
        self.latest_persisted_block.load(Ordering::Relaxed)
    }

    pub fn set_latest_persisted_block(&self, block_number: i64) {
        self.latest_persisted_block
            .store(block_number, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_state_initial_values_and_setters() {
        let state = SharedState::new();
        assert_eq!(state.get_latest_persisted_block(), -1);
        state.set_latest_persisted_block(42);
        assert_eq!(state.get_latest_persisted_block(), 42);

        // active_sessions and block_winners should be empty initially
        assert_eq!(state.active_sessions.len(), 0);
        assert_eq!(state.block_winners.len(), 0);
    }
}
