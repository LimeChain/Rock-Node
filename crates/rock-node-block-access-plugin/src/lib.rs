use async_trait::async_trait;
use rock_node_core::{
    app_context::AppContext,
    error::{Error as CoreError, Result as CoreResult},
    plugin::Plugin,
    BlockReaderProvider,
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
use tonic::service::RoutesBuilder;
use tracing::{info, warn};

mod service;

#[derive(Debug, Default)]
pub struct BlockAccessPlugin {
    context: Option<AppContext>,
    running: Arc<AtomicBool>,
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

    fn initialize(&mut self, context: AppContext) -> CoreResult<()> {
        info!("BlockAccessPlugin initializing...");
        self.context = Some(context);
        Ok(())
    }

    fn start(&mut self) -> CoreResult<()> {
        let context = self.context.as_ref().ok_or_else(|| {
            CoreError::PluginInitialization("BlockAccessPlugin not initialized".to_string())
        })?;
        if context.config.plugins.block_access_service.enabled {
            self.running.store(true, Ordering::SeqCst);
        }
        Ok(())
    }

    fn register_grpc_services(&mut self, builder: &mut RoutesBuilder) -> CoreResult<bool> {
        let context = self.context.as_ref().ok_or_else(|| {
            CoreError::PluginInitialization("BlockAccessPlugin not initialized".to_string())
        })?;

        if !context.config.plugins.block_access_service.enabled {
            return Ok(false);
        }

        let block_reader = {
            let providers = context.service_providers.read().map_err(|_| {
                CoreError::PluginInitialization("Failed to lock providers".to_string())
            })?;
            match providers
                .get(&TypeId::of::<BlockReaderProvider>())
                .and_then(|p| p.downcast_ref::<BlockReaderProvider>())
                .map(|p| p.get_reader())
            {
                Some(reader) => reader,
                None => {
                    warn!(
                        "BlockReaderProvider not found. BlockAccessService will not be available."
                    );
                    return Ok(false);
                },
            }
        };

        let service = BlockAccessServiceImpl {
            block_reader,
            metrics: context.metrics.clone(),
        };
        builder.add_service(BlockAccessServiceServer::new(service));

        Ok(true)
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    async fn stop(&mut self) -> CoreResult<()> {
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
            tx_block_verification_failed: tokio::sync::broadcast::channel(100).0,
            tx_block_persisted: tokio::sync::broadcast::channel(100).0,
        }
    }

    #[test]
    fn test_plugin_name() {
        let plugin = BlockAccessPlugin::new();
        assert_eq!(plugin.name(), "block-access-plugin");
    }

    #[tokio::test]
    async fn test_plugin_registers_services_when_enabled() {
        let mut plugin = BlockAccessPlugin::new();
        let context = create_test_context(true);
        plugin.initialize(context).unwrap();

        let mut builder = RoutesBuilder::default();
        let result = plugin.register_grpc_services(&mut builder).unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn test_plugin_does_not_register_services_when_disabled() {
        let mut plugin = BlockAccessPlugin::new();
        let context = create_test_context(false);
        plugin.initialize(context).unwrap();

        let mut builder = RoutesBuilder::default();
        let result = plugin.register_grpc_services(&mut builder).unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn test_enabled_plugin_lifecycle() {
        let mut plugin = BlockAccessPlugin::new();
        let context = create_test_context(true);

        plugin.initialize(context).unwrap();

        let mut builder = RoutesBuilder::default();
        assert!(plugin.register_grpc_services(&mut builder).unwrap());

        plugin.start().unwrap();
        assert!(plugin.is_running());

        plugin.stop().await.unwrap();
        assert!(!plugin.is_running());
    }
}
