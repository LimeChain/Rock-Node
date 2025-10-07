use thiserror::Error;

/// Errors that can occur during publish operations
#[derive(Error, Debug)]
pub enum PublishError {
    #[error("Block item set is empty")]
    EmptyBlockItems,

    #[error("Missing block header for new block {block_number}")]
    MissingBlockHeader { block_number: i64 },

    #[error("Too many items in set: {count} (max: {max})")]
    TooManyItems { count: usize, max: usize },

    #[error("Block number mismatch: expected {expected}, got {received}")]
    BlockNumberMismatch { expected: i64, received: i64 },

    #[error("Invalid block number: {block_number}")]
    InvalidBlockNumber { block_number: i64 },

    #[error("Session {session_id} received items without active block")]
    NoActiveBlock { session_id: uuid::Uuid },

    #[error("Failed to encode block: {0}")]
    Encoding(#[from] prost::EncodeError),

    #[error("Channel send error")]
    ChannelSend,

    #[error("Verification timeout for block {block_number}")]
    VerificationTimeout { block_number: u64 },

    #[error("Verification failed for block {block_number}: {reason}")]
    VerificationFailed { block_number: u64, reason: String },
}

pub type Result<T> = std::result::Result<T, PublishError>;
