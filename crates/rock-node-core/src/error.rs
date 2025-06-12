use thiserror::Error;

/// A top-level error type for the Rock Node core library and application.
#[derive(Debug, Error)]
pub enum Error {
    #[error("Configuration Error: {0}")]
    Configuration(String),

    #[error("Plugin Initialization Failed: {0}")]
    PluginInitialization(String),

    #[error("I/O Error")]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// A specialized Result type for core operations.
pub type Result<T> = std::result::Result<T, Error>; 