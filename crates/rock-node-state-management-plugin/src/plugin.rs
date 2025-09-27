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
}

impl StateManagementPlugin {
    pub fn new() -> Self {
        Self {
            app_context: None,
            state_manager: None,
            running: Arc::new(AtomicBool::new(false)),
            shutdown_notify: Arc::new(Notify::new()),
        }
    }
}

#[async_trait]
impl Plugin for StateManagementPlugin {
    fn name(&self) -> &'static str {
        "rock-node-state-management-plugin"
    }

    fn initialize(&mut self, context: AppContext) -> CoreResult<()> {
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
