use crate::app_context::AppContext;
use crate::error::Result;
use async_trait::async_trait;
use tonic::service::RoutesBuilder;

#[async_trait]
pub trait Plugin: Send + Sync {
    /// A unique, machine-readable name for the plugin.
    fn name(&self) -> &'static str;

    /// Called at startup to initialize the plugin.
    fn initialize(&mut self, context: AppContext) -> Result<()>;

    /// Called after all plugins are initialized.
    fn start(&mut self) -> Result<()>;

    /// If the plugin provides gRPC services, this method adds them to the main RoutesBuilder.
    /// It should return `Ok(true)` if services were added, and `Ok(false)` otherwise.
    fn register_grpc_services(&mut self, _builder: &mut RoutesBuilder) -> Result<bool> {
        Ok(false)
    }

    /// Returns true if the plugin's primary tasks are running.
    fn is_running(&self) -> bool;

    /// Signals the plugin to gracefully shut down its tasks.
    async fn stop(&mut self) -> Result<()>;
}
