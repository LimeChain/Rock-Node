use thiserror::Error;

/// A top-level error type for the Rock Node core library and application.
#[derive(Debug, Error)]
pub enum Error {
    #[error("Configuration Error: {0}")]
    Configuration(String),

    #[error("Plugin Initialization Failed: {0}")]
    PluginInitialization(String),

    #[error("Plugin Shutdown Failed: {0}")]
    PluginShutdown(String),

    #[error("I/O Error")]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// A specialized Result type for core operations.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn test_configuration_error_display() {
        let error = Error::Configuration("invalid config value".to_string());
        assert_eq!(
            error.to_string(),
            "Configuration Error: invalid config value"
        );
    }

    #[test]
    fn test_plugin_initialization_error_display() {
        let error = Error::PluginInitialization("failed to load plugin".to_string());
        assert_eq!(
            error.to_string(),
            "Plugin Initialization Failed: failed to load plugin"
        );
    }

    #[test]
    fn test_plugin_shutdown_error_display() {
        let error = Error::PluginShutdown("failed to cleanup resources".to_string());
        assert_eq!(
            error.to_string(),
            "Plugin Shutdown Failed: failed to cleanup resources"
        );
    }

    #[test]
    fn test_io_error_conversion() {
        let io_error = io::Error::new(io::ErrorKind::NotFound, "file not found");
        let error: Error = io_error.into();

        match error {
            Error::Io(_) => assert!(true),
            _ => panic!("Expected Error::Io variant"),
        }
    }

    #[test]
    fn test_io_error_display() {
        let io_error = io::Error::new(io::ErrorKind::PermissionDenied, "access denied");
        let error: Error = io_error.into();
        assert_eq!(error.to_string(), "I/O Error");
    }

    #[test]
    fn test_anyhow_error_conversion() {
        let anyhow_error = anyhow::anyhow!("something went wrong");
        let error: Error = anyhow_error.into();

        match error {
            Error::Other(_) => assert!(true),
            _ => panic!("Expected Error::Other variant"),
        }
    }

    #[test]
    fn test_anyhow_error_message_preserved() {
        let anyhow_error = anyhow::anyhow!("custom error message");
        let error: Error = anyhow_error.into();
        assert_eq!(error.to_string(), "custom error message");
    }

    #[test]
    fn test_result_type_ok() {
        let result: Result<i32> = Ok(42);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
    }

    #[test]
    fn test_result_type_err() {
        let result: Result<i32> = Err(Error::Configuration("test error".to_string()));
        assert!(result.is_err());
    }

    #[test]
    fn test_error_debug_format() {
        let error = Error::Configuration("debug test".to_string());
        let debug_str = format!("{:?}", error);
        assert!(debug_str.contains("Configuration"));
        assert!(debug_str.contains("debug test"));
    }

    #[test]
    fn test_io_error_from_different_kinds() {
        let kinds = vec![
            io::ErrorKind::NotFound,
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::ConnectionRefused,
            io::ErrorKind::ConnectionReset,
            io::ErrorKind::TimedOut,
            io::ErrorKind::WriteZero,
            io::ErrorKind::Interrupted,
            io::ErrorKind::UnexpectedEof,
        ];

        for kind in kinds {
            let io_error = io::Error::new(kind, "test error");
            let error: Error = io_error.into();
            assert!(matches!(error, Error::Io(_)));
        }
    }

    #[test]
    fn test_error_send_sync() {
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}
        assert_send::<Error>();
        assert_sync::<Error>();
    }

    #[test]
    fn test_nested_anyhow_context() {
        let base_error = anyhow::anyhow!("base error");
        let contextual_error = base_error.context("additional context");
        let error: Error = contextual_error.into();

        let error_string = error.to_string();
        assert!(error_string.contains("additional context"));
    }
}
