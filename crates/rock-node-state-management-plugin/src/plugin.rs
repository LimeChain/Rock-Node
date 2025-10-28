use crate::state_manager::StateManager;
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use rock_node_core::{
    app_context::AppContext,
    database_provider::DatabaseManagerProvider,
    error::{Error as CoreError, Result as CoreResult},
    plugin::Plugin,
    state_reader::StateReaderProvider,
    BlockReaderProvider,
};
use std::{
    any::TypeId,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::sync::Notify;
use tracing::{error, info, warn};

pub struct StateManagementPlugin {
    app_context: Option<AppContext>,
    state_manager: Option<Arc<StateManager>>,
    running: Arc<AtomicBool>,
    shutdown_notify: Arc<Notify>,
    enabled: bool,
}

impl Default for StateManagementPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl StateManagementPlugin {
    pub fn new() -> Self {
        Self {
            app_context: None,
            state_manager: None,
            running: Arc::new(AtomicBool::new(false)),
            shutdown_notify: Arc::new(Notify::new()),
            enabled: false,
        }
    }
}

#[async_trait]
impl Plugin for StateManagementPlugin {
    fn name(&self) -> &'static str {
        "rock-node-state-management-plugin"
    }

    fn initialize(&mut self, context: AppContext) -> CoreResult<()> {
        self.enabled = context.config.plugins.state_management_service.enabled;

        if !self.enabled {
            info!("StateManagementPlugin is disabled via configuration; skipping initialization.");
            self.app_context = Some(context);
            return Ok(());
        }

        self.init_internal(context)
            .map_err(|e| CoreError::PluginInitialization(e.to_string()))
    }

    fn start(&mut self) -> CoreResult<()> {
        self.start_internal()
            .map_err(|e| CoreError::PluginInitialization(e.to_string()))
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    async fn stop(&mut self) -> CoreResult<()> {
        info!("Stopping StateManagementPlugin...");
        self.shutdown_notify.notify_waiters();
        self.running.store(false, Ordering::SeqCst);
        Ok(())
    }
}

impl StateManagementPlugin {
    fn init_internal(&mut self, context: AppContext) -> Result<()> {
        info!("Initializing State Management Plugin...");
        let providers = context
            .service_providers
            .read()
            .map_err(|_| anyhow!("Failed to acquire read lock on service providers"))?;

        let db_provider = providers
            .get(&TypeId::of::<DatabaseManagerProvider>())
            .and_then(|p| p.downcast_ref::<DatabaseManagerProvider>())
            .cloned()
            .context("DatabaseManagerProvider not found")?;

        let block_reader = providers
            .get(&TypeId::of::<BlockReaderProvider>())
            .and_then(|p| p.downcast_ref::<BlockReaderProvider>())
            .map(|p_concrete| p_concrete.get_reader())
            .ok_or_else(|| anyhow!("BlockReaderProvider not found"))?;

        let db_manager = db_provider.get_manager();
        let cache = context.block_data_cache.clone();

        let state_manager = Arc::new(StateManager::new(db_manager, cache, block_reader));
        self.state_manager = Some(state_manager.clone());

        let reader_provider = StateReaderProvider::new(state_manager);
        drop(providers);
        context
            .service_providers
            .write()
            .map_err(|_| anyhow!("Failed to acquire write lock on service providers"))?
            .insert(
                TypeId::of::<StateReaderProvider>(),
                Arc::new(reader_provider),
            );

        info!("StateReaderProvider registered successfully.");
        self.app_context = Some(context);
        Ok(())
    }

    fn start_internal(&mut self) -> Result<()> {
        let context = self
            .app_context
            .clone()
            .context("AppContext not initialized")?;

        if !self.enabled {
            info!("StateManagementPlugin start skipped because it is disabled via configuration.");
            return Ok(());
        }

        // Check if start_block_number is non-zero
        if context.config.core.start_block_number > 0 {
            warn!("StateManagementPlugin is disabled because the configured start_block_number is greater than 0.");
            warn!("The plugin will be able to restore from a state snapshot in a future update.");
            // TODO: Implement state snapshot restoration to support non-zero genesis blocks.
            return Ok(());
        }

        let state_manager = self
            .state_manager
            .clone()
            .context("StateManager not initialized")?;
        let shutdown_notify = self.shutdown_notify.clone();
        let running_clone = self.running.clone();

        self.running.store(true, Ordering::SeqCst);
        tokio::spawn(async move {
            let mut last_processed_block = match state_manager.get_last_processed_block() {
                Ok(val) => val,
                Err(e) => {
                    error!("Could not read last processed block: {:?}. Halting.", e);
                    running_clone.store(false, Ordering::SeqCst);
                    return;
                },
            };

            if let Some(last) = last_processed_block {
                info!("Resuming state processing. Last processed block: {}.", last);
            } else {
                info!("Starting state processing from genesis block 0.");
            }

            let mut rx = context.tx_block_persisted.subscribe();
            loop {
                tokio::select! {
                    _ = shutdown_notify.notified() => {
                        info!("StateManagementPlugin received shutdown signal. Exiting loop.");
                        break;
                    }
                    event_res = rx.recv() => {
                        match event_res {
                            Ok(event) => {
                                let expected_block = last_processed_block.map_or(0, |n| n + 1);
                                if event.block_number < expected_block {
                                    continue;
                                }
                                if event.block_number > expected_block {
                                    warn!("State fell behind. Expected {}, got {}. Attempting to catch up.", expected_block, event.block_number);
                                    for b in expected_block..event.block_number {
                                        if let Err(e) = state_manager.apply_state_from_storage(b).await {
                                            error!("Failed to apply state for block #{} from storage: {:?}. Halting.", b, e);
                                            break;
                                        }
                                    }
                                }
                                if let Err(e) = state_manager.apply_state_from_block_event(event).await {
                                    error!("Failed to apply state for block event {}: {:?}. Halting.", event.block_number, e);
                                    break;
                                }
                                last_processed_block = Some(event.block_number);
                            }
                            Err(e) => {
                                error!("State plugin event channel error: {:?}. Shutting down.", e);
                                break;
                            }
                        }
                    }
                }
            }
            running_clone.store(false, Ordering::SeqCst);
            info!("StateManagementPlugin event loop has terminated.");
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rock_node_core::{
        app_context::AppContext,
        cache::BlockDataCache,
        capability::CapabilityRegistry,
        config::{Config, CoreConfig, PluginConfigs, StateManagementServiceConfig},
        state_reader::StateReaderProvider,
        test_utils::create_isolated_metrics,
    };
    use std::{
        any::TypeId,
        collections::HashMap,
        sync::{Arc, RwLock},
    };
    use tokio::sync::{broadcast, mpsc};

    fn create_test_context(enabled: bool) -> AppContext {
        let config = Config {
            core: CoreConfig::default(),
            plugins: PluginConfigs {
                state_management_service: StateManagementServiceConfig { enabled },
                ..Default::default()
            },
        };

        AppContext {
            config: Arc::new(config),
            metrics: Arc::new(create_isolated_metrics()),
            capability_registry: Arc::new(CapabilityRegistry::new()),
            service_providers: Arc::new(RwLock::new(HashMap::new())),
            block_data_cache: Arc::new(BlockDataCache::new()),
            tx_block_items_received: mpsc::channel(16).0,
            tx_block_verified: mpsc::channel(16).0,
            tx_block_verification_failed: broadcast::channel(16).0,
            tx_block_persisted: broadcast::channel(16).0,
        }
    }

    #[tokio::test]
    async fn test_initialize_disabled_skips_internal_setup() {
        let ctx = create_test_context(false);
        let providers = ctx.service_providers.clone();

        let mut plugin = StateManagementPlugin::new();
        plugin.initialize(ctx.clone()).unwrap();

        assert!(!plugin.enabled, "plugin should record disabled state");
        assert!(
            plugin.state_manager.is_none(),
            "state manager should not be created"
        );
        assert!(
            providers
                .read()
                .unwrap()
                .get(&TypeId::of::<StateReaderProvider>())
                .is_none(),
            "StateReaderProvider should not be registered when disabled"
        );
        assert!(
            !plugin.is_running(),
            "plugin must not be running after initialization"
        );
    }

    #[tokio::test]
    async fn test_start_disabled_does_not_run() {
        let ctx = create_test_context(false);
        let mut plugin = StateManagementPlugin::new();

        plugin.initialize(ctx).unwrap();
        plugin.start().unwrap();
        assert!(
            !plugin.is_running(),
            "plugin should remain stopped when disabled via config"
        );
    }
}
