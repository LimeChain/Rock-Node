use crate::app_context::AppContext;
use crate::error::Result;

/// The central trait that all plugins must implement.
/// It defines the lifecycle hooks for a plugin.
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
    fn start(&self) -> Result<()>;
}