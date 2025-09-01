use async_trait::async_trait;
use rock_node_core::{
    app_context::AppContext, error::Result, plugin::Plugin, BlockReaderProvider, Error as CoreError,
};
use rock_node_protobufs::org::hiero::block::api::block_access_service_server::BlockAccessServiceServer;
use service::BlockAccessServiceImpl;
use std::{
    any::TypeId,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tonic::transport::server::Router;
use tracing::{info, warn};

mod service;

#[derive(Debug, Default)]
pub struct BlockAccessPlugin {
    running: Arc<AtomicBool>,
    router: Option<Router>,
}

impl BlockAccessPlugin {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Plugin for BlockAccessPlugin {
    fn name(&self) -> &'static str {
        "block-access-plugin"
    }

    fn initialize(&mut self, context: AppContext) -> Result<()> {
        info!("BlockAccessPlugin initializing...");
        if !context.config.plugins.block_access_service.enabled {
            info!("BlockAccessPlugin is disabled.");
            return Ok(());
        }

        let block_reader = {
            let providers = context.service_providers.read().map_err(|_| {
                CoreError::PluginInitialization("Failed to lock providers".to_string())
            })?;
            providers
                .get(&TypeId::of::<BlockReaderProvider>())
                .and_then(|p| p.downcast_ref::<BlockReaderProvider>())
                .map(|p| p.get_reader())
                .ok_or_else(|| {
                    warn!("BlockReaderProvider not found. Service will not be available.");
                    CoreError::PluginInitialization("BlockReaderProvider not found".to_string())
                })?
        };

        let service = BlockAccessServiceImpl {
            block_reader,
            metrics: context.metrics.clone(),
        };
        let server = BlockAccessServiceServer::new(service);
        self.router = Some(tonic::transport::Server::builder().add_service(server));

        Ok(())
    }

    fn start(&mut self) -> Result<()> {
        // The gRPC server is started by main; this just marks the plugin as running.
        self.running.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn take_grpc_router(&mut self) -> Option<Router> {
        self.router.take()
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    async fn stop(&mut self) -> Result<()> {
        self.running.store(false, Ordering::SeqCst);
        Ok(())
    }
}

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
            BackfillConfig, BlockAccessServiceConfig, Config, CoreConfig, ObservabilityConfig,
            PluginConfigs,
        },
        test_utils::create_isolated_metrics,
    };
    use std::{
        any::TypeId,
        collections::HashMap,
        sync::{Arc, RwLock},
    };

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
                block_access_service: BlockAccessServiceConfig { enabled },
                backfill: BackfillConfig::default(),
                server_status_service: Default::default(),
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
        let plugin = BlockAccessPlugin::new();
        assert_eq!(plugin.name(), "block-access-plugin");
    }

    #[tokio::test]
    async fn test_plugin_initialization_creates_router() {
        let mut plugin = BlockAccessPlugin::new();
        let context = create_test_context(true);

        let result = plugin.initialize(context);
        assert!(result.is_ok());
        assert!(!plugin.is_running());
        assert!(plugin.router.is_some());
    }

    #[test]
    fn test_start_without_initialization_is_ok() {
        let mut plugin = BlockAccessPlugin::new();
        // Since start no longer accesses context, it doesn't need to be initialized to run
        let result = plugin.start();
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_start_disabled_plugin() {
        let mut plugin = BlockAccessPlugin::new();
        let context = create_test_context(false); // Disabled

        plugin.initialize(context).unwrap();
        assert!(plugin.router.is_none()); // Router is not created if disabled

        let result = plugin.start();
        assert!(result.is_ok());
        assert!(!plugin.is_running()); // Should not be set to running if disabled
    }

    #[tokio::test]
    async fn test_enabled_plugin_lifecycle() {
        let mut plugin = BlockAccessPlugin::new();
        let context = create_test_context(true);

        plugin.initialize(context).unwrap();
        assert!(plugin.router.is_some());

        let result = plugin.start();
        assert!(result.is_ok());
        assert!(plugin.is_running());

        let router = plugin.take_grpc_router();
        assert!(router.is_some());
        assert!(plugin.router.is_none()); // Router should be gone after take

        plugin.stop().await.unwrap();
        assert!(!plugin.is_running());
    }
}
