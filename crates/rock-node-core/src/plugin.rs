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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_context::AppContext;
    use crate::config::{Config, CoreConfig, PluginConfigs};
    use crate::error::Error;
    use crate::test_utils::create_isolated_metrics;
    use std::any::TypeId;
    use std::collections::HashMap;
    use std::sync::{Arc, RwLock};
    use tokio::sync::{broadcast, mpsc};
    use tonic::service::RoutesBuilder;

    // Mock plugin implementations for testing
    #[derive(Debug)]
    struct MockPlugin {
        name: &'static str,
        initialized: bool,
        started: bool,
        stopped: bool,
        should_fail_init: bool,
        should_fail_start: bool,
        should_fail_stop: bool,
        should_register_grpc: bool,
    }

    impl MockPlugin {
        fn new(name: &'static str) -> Self {
            Self {
                name,
                initialized: false,
                started: false,
                stopped: false,
                should_fail_init: false,
                should_fail_start: false,
                should_fail_stop: false,
                should_register_grpc: false,
            }
        }

        fn with_init_failure(mut self) -> Self {
            self.should_fail_init = true;
            self
        }

        fn with_start_failure(mut self) -> Self {
            self.should_fail_start = true;
            self
        }

        fn with_stop_failure(mut self) -> Self {
            self.should_fail_stop = true;
            self
        }

        fn with_grpc_services(mut self) -> Self {
            self.should_register_grpc = true;
            self
        }
    }

    #[async_trait]
    impl Plugin for MockPlugin {
        fn name(&self) -> &'static str {
            self.name
        }

        fn initialize(&mut self, _context: AppContext) -> Result<()> {
            if self.should_fail_init {
                return Err(Error::PluginInitialization("Mock initialization failure".to_string()));
            }
            self.initialized = true;
            Ok(())
        }

        fn start(&mut self) -> Result<()> {
            if self.should_fail_start {
                return Err(Error::PluginInitialization("Mock start failure".to_string()));
            }
            if !self.initialized {
                return Err(Error::PluginInitialization("Plugin not initialized".to_string()));
            }
            self.started = true;
            Ok(())
        }

        fn register_grpc_services(&mut self, _builder: &mut RoutesBuilder) -> Result<bool> {
            if self.should_register_grpc {
                Ok(true)
            } else {
                Ok(false)
            }
        }

        fn is_running(&self) -> bool {
            self.started && !self.stopped
        }

        async fn stop(&mut self) -> Result<()> {
            if self.should_fail_stop {
                return Err(Error::PluginShutdown("Mock stop failure".to_string()));
            }
            self.stopped = true;
            Ok(())
        }
    }

    fn create_test_app_context() -> AppContext {
        let config = Config {
            core: CoreConfig {
                log_level: "INFO".to_string(),
                database_path: "/tmp/test".to_string(),
                start_block_number: 0,
                grpc_address: "127.0.0.1".to_string(),
                grpc_port: 8080,
            },
            plugins: PluginConfigs::default(),
        };

        let providers: HashMap<TypeId, Arc<dyn std::any::Any + Send + Sync>> = HashMap::new();

        AppContext {
            config: Arc::new(config),
            metrics: Arc::new(create_isolated_metrics()),
            capability_registry: Arc::new(crate::capability::CapabilityRegistry::new()),
            service_providers: Arc::new(RwLock::new(providers)),
            block_data_cache: Arc::new(crate::cache::BlockDataCache::default()),
            tx_block_items_received: mpsc::channel(100).0,
            tx_block_verified: mpsc::channel(100).0,
            tx_block_persisted: broadcast::channel(100).0,
        }
    }

    #[tokio::test]
    async fn test_plugin_lifecycle_success() {
        let mut plugin = MockPlugin::new("test-plugin");
        let context = create_test_app_context();

        // Test initial state
        assert_eq!(plugin.name(), "test-plugin");
        assert!(!plugin.is_running());

        // Test initialization
        let result = plugin.initialize(context);
        assert!(result.is_ok());
        assert!(plugin.initialized);
        assert!(!plugin.is_running());

        // Test start
        let result = plugin.start();
        assert!(result.is_ok());
        assert!(plugin.started);
        assert!(plugin.is_running());

        // Test stop
        let result = plugin.stop().await;
        assert!(result.is_ok());
        assert!(plugin.stopped);
        assert!(!plugin.is_running());
    }

    #[tokio::test]
    async fn test_plugin_initialization_failure() {
        let mut plugin = MockPlugin::new("failing-plugin").with_init_failure();
        let context = create_test_app_context();

        let result = plugin.initialize(context);
        assert!(result.is_err());
        assert!(!plugin.initialized);
        assert!(!plugin.is_running());

        match result.unwrap_err() {
            Error::PluginInitialization(msg) => {
                assert!(msg.contains("Mock initialization failure"));
            }
            _ => panic!("Expected PluginInitialization error"),
        }
    }

    #[tokio::test]
    async fn test_plugin_start_failure() {
        let mut plugin = MockPlugin::new("failing-plugin").with_start_failure();
        let context = create_test_app_context();

        // Initialize successfully
        plugin.initialize(context).unwrap();

        // Start should fail
        let result = plugin.start();
        assert!(result.is_err());
        assert!(!plugin.started);
        assert!(!plugin.is_running());

        match result.unwrap_err() {
            Error::PluginInitialization(msg) => {
                assert!(msg.contains("Mock start failure"));
            }
            _ => panic!("Expected PluginInitialization error"),
        }
    }

    #[tokio::test]
    async fn test_plugin_start_without_initialization() {
        let mut plugin = MockPlugin::new("uninitialized-plugin");

        // Try to start without initialization
        let result = plugin.start();
        assert!(result.is_err());
        assert!(!plugin.is_running());

        match result.unwrap_err() {
            Error::PluginInitialization(msg) => {
                assert!(msg.contains("Plugin not initialized"));
            }
            _ => panic!("Expected PluginInitialization error"),
        }
    }

    #[tokio::test]
    async fn test_plugin_stop_failure() {
        let mut plugin = MockPlugin::new("failing-plugin").with_stop_failure();
        let context = create_test_app_context();

        // Initialize and start successfully
        plugin.initialize(context).unwrap();
        plugin.start().unwrap();
        assert!(plugin.is_running());

        // Stop should fail
        let result = plugin.stop().await;
        assert!(result.is_err());
        assert!(!plugin.stopped);
        assert!(plugin.is_running()); // Still running because stop failed

        match result.unwrap_err() {
            Error::PluginShutdown(msg) => {
                assert!(msg.contains("Mock stop failure"));
            }
            _ => panic!("Expected PluginShutdown error"),
        }
    }

    #[tokio::test]
    async fn test_plugin_grpc_service_registration_with_services() {
        let mut plugin = MockPlugin::new("grpc-plugin").with_grpc_services();
        let mut builder = RoutesBuilder::default();

        let result = plugin.register_grpc_services(&mut builder);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), true);
    }

    #[tokio::test]
    async fn test_plugin_grpc_service_registration_without_services() {
        let mut plugin = MockPlugin::new("no-grpc-plugin");
        let mut builder = RoutesBuilder::default();

        let result = plugin.register_grpc_services(&mut builder);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), false);
    }

    #[tokio::test]
    async fn test_plugin_is_running_states() {
        let mut plugin = MockPlugin::new("state-plugin");
        let context = create_test_app_context();

        // Initial state
        assert!(!plugin.is_running());

        // After initialization (but not started)
        plugin.initialize(context).unwrap();
        assert!(!plugin.is_running());

        // After start
        plugin.start().unwrap();
        assert!(plugin.is_running());

        // After stop
        plugin.stop().await.unwrap();
        assert!(!plugin.is_running());
    }

    #[tokio::test]
    async fn test_plugin_name_consistency() {
        let plugin1 = MockPlugin::new("test-plugin-1");
        let plugin2 = MockPlugin::new("test-plugin-2");

        assert_eq!(plugin1.name(), "test-plugin-1");
        assert_eq!(plugin2.name(), "test-plugin-2");
        assert_ne!(plugin1.name(), plugin2.name());
    }

    #[tokio::test]
    async fn test_multiple_plugin_instances() {
        let mut plugin1 = MockPlugin::new("plugin-1");
        let mut plugin2 = MockPlugin::new("plugin-2");
        let context1 = create_test_app_context();
        let context2 = create_test_app_context();

        // Initialize both plugins
        plugin1.initialize(context1).unwrap();
        plugin2.initialize(context2).unwrap();

        // Start both plugins
        plugin1.start().unwrap();
        plugin2.start().unwrap();

        // Both should be running independently
        assert!(plugin1.is_running());
        assert!(plugin2.is_running());

        // Stop one plugin
        plugin1.stop().await.unwrap();
        assert!(!plugin1.is_running());
        assert!(plugin2.is_running()); // Other plugin still running

        // Stop second plugin
        plugin2.stop().await.unwrap();
        assert!(!plugin1.is_running());
        assert!(!plugin2.is_running());
    }

    #[tokio::test]
    async fn test_plugin_trait_object() {
        let mut plugin: Box<dyn Plugin> = Box::new(MockPlugin::new("boxed-plugin"));
        let context = create_test_app_context();

        // Test that we can use the plugin through a trait object
        assert_eq!(plugin.name(), "boxed-plugin");
        assert!(plugin.initialize(context).is_ok());
        assert!(plugin.start().is_ok());
        assert!(plugin.is_running());
        assert!(plugin.stop().await.is_ok());
        assert!(!plugin.is_running());
    }

    #[tokio::test]
    async fn test_plugin_send_sync() {
        // This test verifies that Plugin trait is Send + Sync
        let plugin = MockPlugin::new("sync-plugin");

        // Should be able to move across thread boundaries
        let handle = tokio::spawn(async move {
            assert_eq!(plugin.name(), "sync-plugin");
        });

        handle.await.unwrap();
    }
}
