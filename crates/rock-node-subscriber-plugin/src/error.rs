use thiserror::Error;

#[derive(Error, Debug)]
pub enum SubscriberError {
    #[error("Client request validation failed: {0}")]
    ValidationFailed(String),

    #[error("BlockReader service not available")]
    BlockReaderUnavailable,

    #[error("Persistence layer error: {0}")]
    PersistenceError(#[from] anyhow::Error),

    #[error("Client disconnected")]
    ClientDisconnected,
}
