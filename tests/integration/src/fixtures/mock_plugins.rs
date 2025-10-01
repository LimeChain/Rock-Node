use async_trait::async_trait;
use rock_node_core::{app_context::AppContext, error::Error as CoreError, plugin::Plugin, Result};

/// A simple mock plugin for testing
#[derive(Debug)]
pub struct MockPlugin {
    name: &'static str,
    initialized: bool,
    started: bool,
    stopped: bool,
    should_fail_init: bool,
    should_fail_start: bool,
}

impl MockPlugin {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            initialized: false,
            started: false,
            stopped: false,
            should_fail_init: false,
            should_fail_start: false,
        }
    }

    pub fn with_init_failure(mut self) -> Self {
        self.should_fail_init = true;
        self
    }

    pub fn with_start_failure(mut self) -> Self {
        self.should_fail_start = true;
        self
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    pub fn is_started(&self) -> bool {
        self.started
    }

    pub fn is_stopped(&self) -> bool {
        self.stopped
    }
}

#[async_trait]
impl Plugin for MockPlugin {
    fn name(&self) -> &'static str {
        self.name
    }

    fn initialize(&mut self, _context: AppContext) -> Result<()> {
        if self.should_fail_init {
            return Err(CoreError::PluginInitialization(format!(
                "{} intentionally failed to initialize",
                self.name
            )));
        }
        self.initialized = true;
        Ok(())
    }

    fn start(&mut self) -> Result<()> {
        if !self.initialized {
            return Err(CoreError::PluginInitialization(
                "Plugin not initialized".to_string(),
            ));
        }
        if self.should_fail_start {
            return Err(CoreError::PluginInitialization(format!(
                "{} intentionally failed to start",
                self.name
            )));
        }
        self.started = true;
        Ok(())
    }

    fn is_running(&self) -> bool {
        self.started && !self.stopped
    }

    async fn stop(&mut self) -> Result<()> {
        self.stopped = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rock_node_core::config::{Config, CoreConfig, PluginConfigs};
    use rock_node_core::test_utils::create_isolated_metrics;

    use std::collections::HashMap;
    use std::sync::{Arc, RwLock};
    use tokio::sync::{broadcast, mpsc};

    fn create_test_context() -> AppContext {
        let config = Config {
            core: CoreConfig::default(),
            plugins: PluginConfigs::default(),
        };

        AppContext {
            config: Arc::new(config),
            metrics: Arc::new(create_isolated_metrics()),
            capability_registry: Arc::new(rock_node_core::capability::CapabilityRegistry::new()),
            service_providers: Arc::new(RwLock::new(HashMap::new())),
            block_data_cache: Arc::new(rock_node_core::BlockDataCache::new()),
            tx_block_items_received: mpsc::channel(10).0,
            tx_block_verified: mpsc::channel(10).0,
            tx_block_persisted: broadcast::channel(10).0,
        }
    }

    #[tokio::test]
    async fn test_mock_plugin_lifecycle() {
        let mut plugin = MockPlugin::new("test-plugin");
        let context = create_test_context();

        assert!(!plugin.is_initialized());
        assert!(!plugin.is_running());

        plugin.initialize(context).unwrap();
        assert!(plugin.is_initialized());
        assert!(!plugin.is_running());

        plugin.start().unwrap();
        assert!(plugin.is_running());

        plugin.stop().await.unwrap();
        assert!(!plugin.is_running());
        assert!(plugin.is_stopped());
    }

    #[tokio::test]
    async fn test_mock_plugin_init_failure() {
        let mut plugin = MockPlugin::new("failing-plugin").with_init_failure();
        let context = create_test_context();

        let result = plugin.initialize(context);
        assert!(result.is_err());
        assert!(!plugin.is_initialized());
    }

    #[tokio::test]
    async fn test_mock_plugin_start_failure() {
        let mut plugin = MockPlugin::new("failing-plugin").with_start_failure();
        let context = create_test_context();

        plugin.initialize(context).unwrap();
        let result = plugin.start();
        assert!(result.is_err());
        assert!(!plugin.is_running());
    }
}
