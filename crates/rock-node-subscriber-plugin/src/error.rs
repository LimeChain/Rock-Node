use rock_node_protobufs::org::hiero::block::api::subscribe_stream_response::Code;
use thiserror::Error;

#[allow(dead_code)]
#[derive(Error, Debug)]
pub enum SubscriberError {
    #[error("Validation failed: {0}")]
    Validation(String, Code),

    #[error("Persistence layer error: {0}")]
    Persistence(#[from] anyhow::Error),

    #[error("Client disconnected")]
    ClientDisconnected,

    #[error("Live stream fell too far behind")]
    StreamLagged,

    #[error("Timed out waiting for block #{0} to be persisted")]
    TimeoutWaitingForBlock(u64),

    #[error("Internal task error: {0}")]
    Internal(String),
}

impl SubscriberError {
    /// Maps an internal error to the appropriate gRPC status code to send to the client.
    pub fn to_status_code(&self) -> Code {
        match self {
            SubscriberError::Validation(_, code) => *code,
            SubscriberError::Persistence(_) => Code::ReadStreamNotAvailable,
            SubscriberError::StreamLagged => Code::ReadStreamNotAvailable,
            SubscriberError::TimeoutWaitingForBlock(_) => Code::ReadStreamNotAvailable,
            SubscriberError::ClientDisconnected => Code::ReadStreamUnknown,
            SubscriberError::Internal(_) => Code::ReadStreamUnknown,
        }
    }

    /// Maps an internal error to a label for the metrics counter.
    pub fn to_metric_label(&self) -> &'static str {
        match self {
            SubscriberError::Validation(_, _) => "invalid_request",
            SubscriberError::Persistence(_) => "persistence_error",
            SubscriberError::ClientDisconnected => "client_disconnect",
            SubscriberError::StreamLagged => "stream_lagged",
            SubscriberError::TimeoutWaitingForBlock(_) => "timeout",
            SubscriberError::Internal(_) => "internal_error",
        }
    }
}
