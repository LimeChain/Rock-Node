use async_trait::async_trait;
use rock_node_core::{
    app_context::AppContext, error::Result, plugin::Plugin, BlockReaderProvider, Error as CoreError,
};
use rock_node_protobufs::org::hiero::block::api::block_node_service_server::BlockNodeServiceServer;
use service::StatusServiceImpl;
use std::{
    any::TypeId,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::sync::watch;
use tracing::{error, info, warn};

mod service;

//================================================================================//
//=============================== UNIT TESTS =====================================//
//================================================================================//

#[cfg(test)]
mod tests {
    use super::*;
    use rock_node_core::{
        app_context::AppContext,
        block_reader::{BlockReader, BlockReaderProvider},
        config::{
            BackfillConfig, Config, CoreConfig, ObservabilityConfig, PluginConfigs,
            ServerStatusServiceConfig,
        },
        test_utils::create_isolated_metrics,
    };
    use std::{
        any::TypeId,
        collections::HashMap,
        sync::{Arc, RwLock},
    };
    use tokio::time::{sleep, Duration};

    #[derive(Debug, Default)]
    struct MockBlockReader;

    impl BlockReader for MockBlockReader {
        fn read_block(&self, _block_number: u64) -> anyhow::Result<Option<Vec<u8>>> {
            Ok(None)
        }

        fn get_earliest_persisted_block_number(&self) -> anyhow::Result<Option<u64>> {
            Ok(Some(100))
        }

        fn get_latest_persisted_block_number(&self) -> anyhow::Result<Option<u64>> {
            Ok(Some(1000))
        }

        fn get_highest_contiguous_block_number(&self) -> anyhow::Result<u64> {
            Ok(1000)
        }
    }

    fn create_test_context(enabled: bool) -> AppContext {
        let config = Config {
            plugins: PluginConfigs {
                server_status_service: ServerStatusServiceConfig {
                    enabled,
                    grpc_address: "127.0.0.1".to_string(),
                    grpc_port: 0, // Use port 0 to let OS assign a random port
                },
                backfill: BackfillConfig::default(),
                block_access_service: Default::default(),
                observability: ObservabilityConfig::default(),
                persistence_service: Default::default(),
                publish_service: Default::default(),
                verification_service: Default::default(),
                state_management_service: Default::default(),
                subscriber_service: Default::default(),
                query_service: Default::default(),
            },
            core: CoreConfig::default(),
        };

        let mut providers: HashMap<TypeId, Arc<dyn std::any::Any + Send + Sync>> = HashMap::new();
        let mock_reader = Arc::new(MockBlockReader);
        providers.insert(
            TypeId::of::<BlockReaderProvider>(),
            Arc::new(BlockReaderProvider::new(mock_reader)),
        );

        AppContext {
            config: Arc::new(config),
            service_providers: Arc::new(RwLock::new(providers)),
            metrics: Arc::new(create_isolated_metrics()),
            capability_registry: Arc::new(Default::default()),
            block_data_cache: Arc::new(Default::default()),
            tx_block_items_received: tokio::sync::mpsc::channel(100).0,
            tx_block_verified: tokio::sync::mpsc::channel(100).0,
            tx_block_persisted: tokio::sync::broadcast::channel(100).0,
        }
    }

    #[test]
    fn test_plugin_name() {
        let plugin = StatusPlugin::new();
        assert_eq!(plugin.name(), "status-plugin");
    }

    #[tokio::test]
    async fn test_plugin_initialization() {
        let mut plugin = StatusPlugin::new();
        let context = create_test_context(true);

        let result = plugin.initialize(context);
        assert!(result.is_ok());
        assert!(plugin.context.is_some());
        assert!(!plugin.is_running());
    }

    #[tokio::test]
    async fn test_plugin_initialization_twice() {
        let mut plugin = StatusPlugin::new();
        let context = create_test_context(true);

        // First initialization should succeed
        assert!(plugin.initialize(context.clone()).is_ok());

        // Second initialization should still work (or at least not panic)
        assert!(plugin.initialize(context).is_ok());
    }

    #[test]
    fn test_start_without_initialization() {
        let mut plugin = StatusPlugin::new();

        // Starting without initialization should fail
        let result = plugin.start();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("StatusPlugin not initialized"));
    }

    #[tokio::test]
    async fn test_start_disabled_plugin() {
        let mut plugin = StatusPlugin::new();
        let context = create_test_context(false); // Disabled

        plugin.initialize(context).unwrap();
        let result = plugin.start();
        assert!(result.is_ok()); // Should succeed but not actually start
        assert!(!plugin.is_running());
    }

    #[tokio::test]
    async fn test_start_enabled_plugin() {
        let mut plugin = StatusPlugin::new();
        let context = create_test_context(true); // Enabled

        plugin.initialize(context).unwrap();
        let result = plugin.start();
        assert!(result.is_ok());
        assert!(plugin.is_running());

        // Give the server a moment to start
        sleep(Duration::from_millis(100)).await;

        // Clean shutdown
        plugin.stop().await.unwrap();
        assert!(!plugin.is_running());
    }

    #[tokio::test]
    async fn test_stop_without_start() {
        let mut plugin = StatusPlugin::new();
        let context = create_test_context(true);

        plugin.initialize(context).unwrap();

        // Stopping without starting should be fine
        let result = plugin.stop().await;
        assert!(result.is_ok());
        assert!(!plugin.is_running());
    }

    #[tokio::test]
    async fn test_double_stop() {
        let mut plugin = StatusPlugin::new();
        let context = create_test_context(true);

        plugin.initialize(context).unwrap();
        plugin.start().unwrap();
        sleep(Duration::from_millis(100)).await;

        // First stop should succeed
        assert!(plugin.stop().await.is_ok());
        assert!(!plugin.is_running());

        // Second stop should also succeed
        assert!(plugin.stop().await.is_ok());
        assert!(!plugin.is_running());
    }

    #[tokio::test]
    async fn test_plugin_restart() {
        let mut plugin = StatusPlugin::new();
        let context = create_test_context(true);

        plugin.initialize(context).unwrap();

        // Start the plugin
        plugin.start().unwrap();
        assert!(plugin.is_running());
        sleep(Duration::from_millis(100)).await;

        // Stop it
        plugin.stop().await.unwrap();
        assert!(!plugin.is_running());
        sleep(Duration::from_millis(100)).await;

        // Start it again
        plugin.start().unwrap();
        assert!(plugin.is_running());
        sleep(Duration::from_millis(100)).await;

        // Final cleanup
        plugin.stop().await.unwrap();
        assert!(!plugin.is_running());
    }

    #[test]
    fn test_plugin_not_running_by_default() {
        let plugin = StatusPlugin::new();
        assert!(!plugin.is_running());
    }

    #[tokio::test]
    async fn test_graceful_shutdown() {
        let mut plugin = StatusPlugin::new();
        let context = create_test_context(true);

        plugin.initialize(context).unwrap();
        plugin.start().unwrap();
        assert!(plugin.is_running());

        // Give the server time to start
        sleep(Duration::from_millis(200)).await;

        // Request shutdown
        let stop_result = plugin.stop().await;
        assert!(stop_result.is_ok());

        // Give the server time to shutdown gracefully
        sleep(Duration::from_millis(200)).await;
        assert!(!plugin.is_running());
    }
}

#[derive(Debug, Default)]
pub struct StatusPlugin {
    context: Option<AppContext>,
    running: Arc<AtomicBool>,
    shutdown_tx: Option<watch::Sender<()>>,
}

impl StatusPlugin {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Plugin for StatusPlugin {
    fn name(&self) -> &'static str {
        "status-plugin"
    }

    fn initialize(&mut self, context: AppContext) -> Result<()> {
        info!("StatusPlugin initialized.");
        self.context = Some(context);
        self.running = Arc::new(AtomicBool::new(false));
        Ok(())
    }

    fn start(&mut self) -> Result<()> {
        info!("Starting Server Status Plugin...");
        let context = self
            .context
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("StatusPlugin not initialized"))?
            .clone();

        let config = &context.config.plugins.server_status_service;
        if !config.enabled {
            info!("Server StatusPlugin is disabled. Skipping start.");
            return Ok(());
        }

        let block_reader = {
            let providers = context
                .service_providers
                .read()
                .map_err(|_| anyhow::anyhow!("Failed to acquire read lock on service providers"))?;
            let key = TypeId::of::<BlockReaderProvider>();
            if let Some(provider_any) = providers.get(&key) {
                provider_any
                    .downcast_ref::<BlockReaderProvider>()
                    .map(|p| p.get_reader())
                    .ok_or_else(|| {
                        anyhow::anyhow!("FATAL: Failed to downcast BlockReaderProvider.")
                    })
            } else {
                warn!("BlockReaderProvider not found. Status service will not be able to serve blocks.");
                return Ok(());
            }?
        };

        let listen_address = format!("{}:{}", config.grpc_address, config.grpc_port);
        let socket_addr = listen_address.parse().map_err(|e| {
            anyhow::anyhow!(
                "Failed to parse gRPC listen address '{}': {}",
                listen_address,
                e
            )
        })?;

        let service = StatusServiceImpl {
            block_reader,
            metrics: context.metrics.clone(),
        };
        let server = BlockNodeServiceServer::new(service);

        let (shutdown_tx, mut shutdown_rx) = watch::channel(());
        self.shutdown_tx = Some(shutdown_tx);
        let running_clone = self.running.clone();

        self.running.store(true, Ordering::SeqCst);
        tokio::spawn(async move {
            info!("Status gRPC service listening on {}", socket_addr);

            let server_future = tonic::transport::Server::builder()
                .add_service(server)
                .serve_with_shutdown(socket_addr, async move {
                    shutdown_rx.changed().await.ok();
                    info!("Gracefully shutting down Status gRPC server...");
                });

            if let Err(e) = server_future.await {
                error!("Status gRPC server failed: {}", e);
            }
            running_clone.store(false, Ordering::SeqCst);
        });

        Ok(())
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    async fn stop(&mut self) -> Result<()> {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            if shutdown_tx.send(()).is_err() {
                let msg = "Failed to send shutdown signal to Status gRPC server: receiver dropped.";
                error!("{}", msg);
                return Err(CoreError::PluginShutdown(msg.to_string()));
            }
        }
        self.running.store(false, Ordering::SeqCst);
        Ok(())
    }
}
