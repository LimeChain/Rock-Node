use crate::app_context::AppContext;
use crate::error::Result;
use async_trait::async_trait;

/// The central trait that all plugins must implement.
/// It defines the lifecycle hooks for a plugin.
#[async_trait]
pub trait Plugin: Send + Sync {
    /// A unique, machine-readable name for the plugin.
    fn name(&self) -> &'static str;

    /// Called at startup to initialize the plugin.
    /// The plugin can get shared facilities from the AppContext and
    /// register its own capabilities or providers.
    fn initialize(&mut self, context: AppContext) -> Result<()>;

    /// Called after all plugins are initialized.
    /// If the plugin exposes a network service, it should be started here
    /// in a non-blocking fashion (e.g., using `tokio::spawn`).
    fn start(&mut self) -> Result<()>;

    /// Returns true if the plugin's primary tasks are running.
    /// This is used during shutdown to avoid stopping an already stopped plugin.
    fn is_running(&self) -> bool;

    /// Signals the plugin to gracefully shut down its tasks.
    /// This could involve stopping gRPC servers, flushing buffers, or
    /// signaling background threads to exit.
    async fn stop(&mut self) -> Result<()>;
}
