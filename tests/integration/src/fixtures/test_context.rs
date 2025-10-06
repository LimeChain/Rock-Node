use anyhow::Result;
use rock_node_core::{
    app_context::AppContext,
    capability::CapabilityRegistry,
    config::{Config, CoreConfig, PluginConfigs},
    database::DatabaseManager,
    database_provider::DatabaseManagerProvider,
    events::{BlockItemsReceived, BlockPersisted, BlockVerified},
    BlockDataCache, MetricsRegistry, Plugin,
};
use std::{
    any::{Any, TypeId},
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, RwLock},
};
use tempfile::TempDir;
use tokio::sync::{broadcast, mpsc};
use tracing::info;

/// Builder for creating integration test contexts
pub struct IntegrationTestContextBuilder {
    plugins: Vec<Box<dyn Plugin>>,
    config_overrides: HashMap<String, String>,
    database_path: Option<PathBuf>,
    channel_buffer_size: usize,
}

impl Default for IntegrationTestContextBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl IntegrationTestContextBuilder {
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
            config_overrides: HashMap::new(),
            database_path: None,
            channel_buffer_size: 100,
        }
    }

    /// Add a plugin to the test context
    pub fn with_plugin(mut self, plugin: Box<dyn Plugin>) -> Self {
        self.plugins.push(plugin);
        self
    }

    /// Add a config override (e.g., "core.log_level", "INFO")
    pub fn with_config_override(mut self, key: String, value: String) -> Self {
        self.config_overrides.insert(key, value);
        self
    }

    /// Set custom database path (defaults to temp directory)
    pub fn with_database_path(mut self, path: PathBuf) -> Self {
        self.database_path = Some(path);
        self
    }

    /// Set channel buffer size (defaults to 100)
    pub fn with_channel_buffer_size(mut self, size: usize) -> Self {
        self.channel_buffer_size = size;
        self
    }

    /// Build the integration test context
    pub async fn build(self) -> Result<IntegrationTestContext> {
        // Create temp directory for database and data files
        let temp_dir = TempDir::new()?;
        let database_path = self
            .database_path
            .unwrap_or_else(|| temp_dir.path().join("test_db"));

        // Create base config
        let mut config = Config {
            core: CoreConfig {
                log_level: "INFO".to_string(),
                database_path: database_path.to_string_lossy().to_string(),
                start_block_number: 0,
                grpc_address: "127.0.0.1".to_string(),
                grpc_port: 0, // Random port for tests
            },
            plugins: PluginConfigs::default(),
        };

        // Apply config overrides
        for (key, value) in &self.config_overrides {
            match key.as_str() {
                "core.log_level" => config.core.log_level = value.clone(),
                "core.start_block_number" => {
                    config.core.start_block_number = value.parse().unwrap_or(0)
                },
                _ => {}, // Ignore unknown overrides
            }
        }

        // Initialize database
        let db_manager = DatabaseManager::new(database_path.to_str().unwrap())?;
        let db_provider = DatabaseManagerProvider::new(Arc::new(db_manager));

        // Create isolated metrics registry
        let registry = prometheus::Registry::new();
        let metrics = MetricsRegistry::with_registry(registry)?;

        // Create capability registry
        let capability_registry = CapabilityRegistry::new();

        // Create service providers map
        let mut providers: HashMap<TypeId, Arc<dyn Any + Send + Sync>> = HashMap::new();
        providers.insert(
            TypeId::of::<DatabaseManagerProvider>(),
            Arc::new(db_provider),
        );

        // Create block data cache
        let block_data_cache = BlockDataCache::new();

        // Create event channels
        let (tx_block_items_received, rx_block_items_received) =
            mpsc::channel(self.channel_buffer_size);
        let (tx_block_verified, rx_block_verified) = mpsc::channel(self.channel_buffer_size);
        let (tx_block_verification_failed, _rx_block_verification_failed) =
            broadcast::channel(self.channel_buffer_size);
        let (tx_block_persisted, _rx_block_persisted) =
            broadcast::channel(self.channel_buffer_size);

        // Create AppContext
        let context = AppContext {
            config: Arc::new(config),
            metrics: Arc::new(metrics),
            capability_registry: Arc::new(capability_registry),
            service_providers: Arc::new(RwLock::new(providers)),
            block_data_cache: Arc::new(block_data_cache),
            tx_block_items_received,
            tx_block_verified,
            tx_block_verification_failed,
            tx_block_persisted,
        };

        // Create plugin map for easy access
        let mut plugin_map = HashMap::new();
        for plugin in &self.plugins {
            plugin_map.insert(plugin.name(), ());
        }

        Ok(IntegrationTestContext {
            context,
            plugins: self.plugins,
            plugin_map,
            temp_dir,
            rx_block_items_received: Some(rx_block_items_received),
            rx_block_verified: Some(rx_block_verified),
            initialized: false,
            started: false,
        })
    }
}

/// Integration test context with plugins and shared resources
pub struct IntegrationTestContext {
    pub context: AppContext,
    plugins: Vec<Box<dyn Plugin>>,
    plugin_map: HashMap<&'static str, ()>,
    temp_dir: TempDir,
    pub rx_block_items_received: Option<mpsc::Receiver<BlockItemsReceived>>,
    pub rx_block_verified: Option<mpsc::Receiver<BlockVerified>>,
    initialized: bool,
    started: bool,
}

impl IntegrationTestContext {
    /// Create a new builder
    pub fn builder() -> IntegrationTestContextBuilder {
        IntegrationTestContextBuilder::new()
    }

    /// Get the temporary directory path
    pub fn temp_dir(&self) -> &std::path::Path {
        self.temp_dir.path()
    }

    /// Get database path
    pub fn database_path(&self) -> PathBuf {
        PathBuf::from(&self.context.config.core.database_path)
    }

    /// Check if a plugin was added to this context
    pub fn has_plugin(&self, name: &str) -> bool {
        self.plugin_map.contains_key(name)
    }

    /// Get plugin count
    pub fn plugin_count(&self) -> usize {
        self.plugins.len()
    }

    /// Initialize all plugins
    pub fn initialize_plugins(&mut self) -> Result<()> {
        if self.initialized {
            return Ok(());
        }

        info!("Initializing {} plugins...", self.plugins.len());
        for plugin in &mut self.plugins {
            plugin.initialize(self.context.clone())?;
            info!("  ✓ Initialized: {}", plugin.name());
        }
        self.initialized = true;
        Ok(())
    }

    /// Start all plugins
    pub fn start_plugins(&mut self) -> Result<()> {
        if !self.initialized {
            return Err(anyhow::anyhow!("Plugins not initialized"));
        }
        if self.started {
            return Ok(());
        }

        info!("Starting {} plugins...", self.plugins.len());
        for plugin in &mut self.plugins {
            plugin.start()?;
            info!("  ✓ Started: {}", plugin.name());
        }
        self.started = true;
        Ok(())
    }

    /// Stop all plugins
    pub async fn stop_plugins(&mut self) -> Result<()> {
        if !self.started {
            return Ok(());
        }

        info!("Stopping {} plugins...", self.plugins.len());
        for plugin in &mut self.plugins {
            plugin.stop().await?;
            info!("  ✓ Stopped: {}", plugin.name());
        }
        self.started = false;
        Ok(())
    }

    /// Check if all plugins are running
    pub fn all_plugins_running(&self) -> bool {
        self.plugins.iter().all(|p| p.is_running())
    }

    /// Get names of all running plugins
    pub fn running_plugin_names(&self) -> Vec<&'static str> {
        self.plugins
            .iter()
            .filter(|p| p.is_running())
            .map(|p| p.name())
            .collect()
    }

    /// Get names of all stopped plugins
    pub fn stopped_plugin_names(&self) -> Vec<&'static str> {
        self.plugins
            .iter()
            .filter(|p| !p.is_running())
            .map(|p| p.name())
            .collect()
    }

    /// Subscribe to block persisted events
    pub fn subscribe_block_persisted(&self) -> broadcast::Receiver<BlockPersisted> {
        self.context.tx_block_persisted.subscribe()
    }

    /// Take ownership of block items received channel (can only be done once)
    pub fn take_block_items_received_rx(&mut self) -> Option<mpsc::Receiver<BlockItemsReceived>> {
        self.rx_block_items_received.take()
    }

    /// Take ownership of block verified channel (can only be done once)
    pub fn take_block_verified_rx(&mut self) -> Option<mpsc::Receiver<BlockVerified>> {
        self.rx_block_verified.take()
    }
}

impl Drop for IntegrationTestContext {
    fn drop(&mut self) {
        // Ensure plugins are stopped on drop
        if self.started {
            // Can't await in drop, but log a warning
            tracing::warn!("IntegrationTestContext dropped without stopping plugins!");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_builder_creates_context() {
        let ctx = IntegrationTestContext::builder().build().await.unwrap();

        assert_eq!(ctx.plugin_count(), 0);
        assert!(ctx.temp_dir().exists());
        assert!(ctx.database_path().to_string_lossy().contains("test_db"));
    }

    #[tokio::test]
    async fn test_builder_with_config_overrides() {
        let ctx = IntegrationTestContext::builder()
            .with_config_override("core.log_level".to_string(), "DEBUG".to_string())
            .with_config_override("core.start_block_number".to_string(), "42".to_string())
            .build()
            .await
            .unwrap();

        assert_eq!(ctx.context.config.core.log_level, "DEBUG");
        assert_eq!(ctx.context.config.core.start_block_number, 42);
    }

    #[tokio::test]
    async fn test_context_temp_dir_cleanup() {
        let temp_path = {
            let ctx = IntegrationTestContext::builder().build().await.unwrap();
            let path = ctx.temp_dir().to_path_buf();
            assert!(path.exists());
            path
        };

        // After context is dropped, temp dir should be cleaned up
        assert!(!temp_path.exists());
    }
}
