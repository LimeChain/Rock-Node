mod worker;

use crate::worker::BackfillWorker;
use async_trait::async_trait;
use rock_node_core::{
    app_context::AppContext,
    config::BackfillMode,
    error::{Error as CoreError, Result as CoreResult},
    plugin::Plugin,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::Notify;
use tracing::{info, warn};

#[derive(Debug, Default)]
pub struct BackfillPlugin {
    context: Option<AppContext>,
    running: Arc<AtomicBool>,
    shutdown_notify: Arc<Notify>,
}

impl BackfillPlugin {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Plugin for BackfillPlugin {
    fn name(&self) -> &'static str {
        "backfill-plugin"
    }

    fn initialize(&mut self, context: AppContext) -> CoreResult<()> {
        self.context = Some(context);
        info!("BackfillPlugin initialized.");
        Ok(())
    }

    fn start(&mut self) -> CoreResult<()> {
        let context = self.context.as_ref().cloned().ok_or_else(|| {
            CoreError::PluginInitialization("BackfillPlugin not initialized".to_string())
        })?;
        let config = &context.config.plugins.backfill;

        if !config.enabled {
            info!("BackfillPlugin is disabled via configuration.");
            return Ok(());
        }

        if config.peers.is_empty() {
            warn!("BackfillPlugin is enabled but has no peers configured. Disabling plugin.");
            return Ok(());
        }

        let worker = match BackfillWorker::new(context.clone()) {
            Ok(s) => Arc::new(s),
            Err(e) => return Err(CoreError::PluginInitialization(e.to_string())),
        };

        let shutdown_notify = self.shutdown_notify.clone();
        self.running.store(true, Ordering::SeqCst);
        let running_clone = self.running.clone();
        let mode = config.mode.clone();

        tokio::spawn(async move {
            info!("Starting Backfill background task in {:?} mode.", mode);

            match mode {
                BackfillMode::GapFill => {
                    worker.run_gap_fill_loop(shutdown_notify).await;
                }
                BackfillMode::Continuous => {
                    worker.run_continuous_loop(shutdown_notify).await;
                }
            }

            running_clone.store(false, Ordering::SeqCst);
            info!("Backfill task has terminated.");
        });

        Ok(())
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    async fn stop(&mut self) -> CoreResult<()> {
        self.shutdown_notify.notify_waiters();
        self.running.store(false, Ordering::SeqCst);
        info!("BackfillPlugin stopped.");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rock_node_core::{
        config::{BackfillConfig, Config, CoreConfig, PluginConfigs},
        database::DatabaseManager,
        database_provider::DatabaseManagerProvider,
        metrics::MetricsRegistry,
        BlockReaderProvider, BlockWriterProvider,
    };
    use std::any::TypeId;
    use std::collections::HashMap;
    use tempfile::TempDir;

    // Mock implementations
    #[derive(Debug)]
    struct MockBlockReader;

    #[async_trait]
    impl rock_node_core::block_reader::BlockReader for MockBlockReader {
        fn get_latest_persisted_block_number(&self) -> anyhow::Result<Option<u64>> {
            Ok(Some(100))
        }

        fn read_block(&self, _block_number: u64) -> anyhow::Result<Option<Vec<u8>>> {
            Ok(None)
        }

        fn get_earliest_persisted_block_number(&self) -> anyhow::Result<Option<u64>> {
            Ok(None)
        }

        fn get_highest_contiguous_block_number(&self) -> anyhow::Result<u64> {
            Ok(100)
        }
    }

    #[derive(Debug)]
    struct MockBlockWriter;

    #[async_trait]
    impl rock_node_core::block_writer::BlockWriter for MockBlockWriter {
        async fn write_block(
            &self,
            _block: &rock_node_protobufs::com::hedera::hapi::block::stream::Block,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn write_block_batch(
            &self,
            _blocks: &[rock_node_protobufs::com::hedera::hapi::block::stream::Block],
        ) -> anyhow::Result<()> {
            Ok(())
        }
    }

    fn create_test_cache() -> rock_node_core::cache::BlockDataCache {
        // For tests, we'll use the default implementation but handle the runtime issue
        // by running the tests in a tokio runtime
        rock_node_core::cache::BlockDataCache::default()
    }

    fn create_test_context(
        enabled: bool,
        mode: BackfillMode,
        peers: Vec<String>,
    ) -> (AppContext, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let db_manager = DatabaseManager::new(temp_dir.path().to_str().unwrap()).unwrap();

        let config = Config {
            core: CoreConfig {
                log_level: "INFO".to_string(),
                database_path: temp_dir.path().to_str().unwrap().to_string(),
                start_block_number: 0,
            },
            plugins: PluginConfigs {
                backfill: BackfillConfig {
                    enabled,
                    mode,
                    peers,
                    check_interval_seconds: 1,
                    max_batch_size: 100,
                },
                ..Default::default()
            },
        };

        let mut providers: HashMap<TypeId, Arc<dyn std::any::Any + Send + Sync>> = HashMap::new();

        providers.insert(
            TypeId::of::<BlockReaderProvider>(),
            Arc::new(BlockReaderProvider::new(Arc::new(MockBlockReader))),
        );

        providers.insert(
            TypeId::of::<BlockWriterProvider>(),
            Arc::new(BlockWriterProvider::new(Arc::new(MockBlockWriter))),
        );

        providers.insert(
            TypeId::of::<DatabaseManagerProvider>(),
            Arc::new(DatabaseManagerProvider::new(Arc::new(db_manager))),
        );

        let context = AppContext {
            config: Arc::new(config),
            service_providers: Arc::new(std::sync::RwLock::new(providers)),
            metrics: Arc::new(MetricsRegistry::new().unwrap()),
            capability_registry: Arc::new(rock_node_core::capability::CapabilityRegistry::new()),
            block_data_cache: Arc::new(create_test_cache()),
            tx_block_items_received: tokio::sync::mpsc::channel(100).0,
            tx_block_verified: tokio::sync::mpsc::channel(100).0,
            tx_block_persisted: tokio::sync::broadcast::channel(100).0,
        };

        (context, temp_dir)
    }

    #[test]
    fn test_plugin_new() {
        let plugin = BackfillPlugin::new();
        assert_eq!(plugin.name(), "backfill-plugin");
        assert!(plugin.context.is_none());
        assert!(!plugin.is_running());
    }

    #[test]
    fn test_plugin_default() {
        let plugin = BackfillPlugin::default();
        assert_eq!(plugin.name(), "backfill-plugin");
        assert!(plugin.context.is_none());
        assert!(!plugin.is_running());
    }

    #[tokio::test]
    async fn test_plugin_initialize() {
        let mut plugin = BackfillPlugin::new();
        let (context, _temp) = create_test_context(
            true,
            BackfillMode::GapFill,
            vec!["http://localhost:8080".to_string()],
        );

        let result = plugin.initialize(context.clone());
        assert!(result.is_ok());
        assert!(plugin.context.is_some());
    }

    #[tokio::test]
    async fn test_plugin_start_when_disabled() {
        let mut plugin = BackfillPlugin::new();
        let (context, _temp) = create_test_context(
            false,
            BackfillMode::GapFill,
            vec!["http://localhost:8080".to_string()],
        );

        plugin.initialize(context).unwrap();
        let result = plugin.start();
        assert!(result.is_ok());
        assert!(!plugin.is_running());
    }

    #[tokio::test]
    async fn test_plugin_start_with_no_peers() {
        let mut plugin = BackfillPlugin::new();
        let (context, _temp) = create_test_context(true, BackfillMode::GapFill, vec![]);

        plugin.initialize(context).unwrap();
        let result = plugin.start();
        assert!(result.is_ok());
        assert!(!plugin.is_running());
    }

    #[test]
    fn test_plugin_start_without_initialization() {
        let mut plugin = BackfillPlugin::new();
        let result = plugin.start();
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CoreError::PluginInitialization(_)
        ));
    }

    #[tokio::test]
    async fn test_plugin_start_gap_fill_mode() {
        let mut plugin = BackfillPlugin::new();
        let (context, _temp) = create_test_context(
            true,
            BackfillMode::GapFill,
            vec!["http://localhost:8080".to_string()],
        );

        plugin.initialize(context).unwrap();
        let result = plugin.start();
        assert!(result.is_ok());
        assert!(plugin.is_running());

        // Clean up
        let _ = plugin.stop().await;
    }

    #[tokio::test]
    async fn test_plugin_start_continuous_mode() {
        let mut plugin = BackfillPlugin::new();
        let (context, _temp) = create_test_context(
            true,
            BackfillMode::Continuous,
            vec!["http://localhost:8080".to_string()],
        );

        plugin.initialize(context).unwrap();
        let result = plugin.start();
        assert!(result.is_ok());
        assert!(plugin.is_running());

        // Clean up
        let _ = plugin.stop().await;
    }

    #[tokio::test]
    async fn test_plugin_stop() {
        let mut plugin = BackfillPlugin::new();
        let (context, _temp) = create_test_context(
            true,
            BackfillMode::GapFill,
            vec!["http://localhost:8080".to_string()],
        );

        plugin.initialize(context).unwrap();
        plugin.start().unwrap();
        assert!(plugin.is_running());

        let result = plugin.stop().await;
        assert!(result.is_ok());
        assert!(!plugin.is_running());
    }

    #[tokio::test]
    async fn test_plugin_lifecycle() {
        let mut plugin = BackfillPlugin::new();
        let (context, _temp) = create_test_context(
            true,
            BackfillMode::GapFill,
            vec!["http://localhost:8080".to_string()],
        );

        // Initialize
        assert!(plugin.initialize(context).is_ok());
        assert!(!plugin.is_running());

        // Start
        assert!(plugin.start().is_ok());
        assert!(plugin.is_running());

        // Stop
        assert!(plugin.stop().await.is_ok());
        assert!(!plugin.is_running());
    }

    #[tokio::test]
    async fn test_plugin_with_multiple_peers() {
        let mut plugin = BackfillPlugin::new();
        let peers = vec![
            "http://peer1:8080".to_string(),
            "http://peer2:8080".to_string(),
            "http://peer3:8080".to_string(),
        ];
        let (context, _temp) = create_test_context(true, BackfillMode::Continuous, peers.clone());

        plugin.initialize(context.clone()).unwrap();
        assert_eq!(
            plugin
                .context
                .as_ref()
                .unwrap()
                .config
                .plugins
                .backfill
                .peers,
            peers
        );

        let result = plugin.start();
        assert!(result.is_ok());
        assert!(plugin.is_running());

        // Clean up
        let _ = plugin.stop().await;
    }

    #[test]
    fn test_plugin_debug_format() {
        let plugin = BackfillPlugin::new();
        let debug_str = format!("{:?}", plugin);
        assert!(debug_str.contains("BackfillPlugin"));
    }
}
