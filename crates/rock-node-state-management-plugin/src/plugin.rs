use crate::state_manager::StateManager;
use anyhow::{Context, Result};
use rock_node_core::{
    block_reader::BlockReaderProvider, database_provider::DatabaseManagerProvider,
    state_reader::StateReaderProvider, AppContext, Plugin,
};
use std::any::TypeId;
use std::sync::Arc;
use tracing::{error, info, warn};

pub struct StateManagementPlugin {
    app_context: Option<AppContext>,
    state_manager: Option<Arc<StateManager>>,
}

impl StateManagementPlugin {
    pub fn new() -> Self {
        Self {
            app_context: None,
            state_manager: None,
        }
    }
}

impl Plugin for StateManagementPlugin {
    fn name(&self) -> &'static str {
        "rock-node-state-management-plugin"
    }

    fn initialize(&mut self, context: AppContext) -> rock_node_core::Result<()> {
        self.init_internal(context)
            .map_err(|e| rock_node_core::Error::PluginInitialization(e.to_string()))
    }

    fn start(&mut self) -> rock_node_core::Result<()> {
        self.start_internal()
            .map_err(|e| rock_node_core::Error::PluginInitialization(e.to_string()))
    }
}

impl StateManagementPlugin {
    fn init_internal(&mut self, context: AppContext) -> Result<()> {
        info!("Initializing State Management Plugin...");
        let providers = context.service_providers.read().unwrap();

        let db_provider = providers
            .get(&TypeId::of::<DatabaseManagerProvider>())
            .and_then(|p| p.downcast_ref::<DatabaseManagerProvider>())
            .cloned()
            .context("DatabaseManagerProvider not found")?;

        // Fetch the BlockReaderProvider to use for catch-up logic.
        let block_reader_provider = providers
            .get(&TypeId::of::<BlockReaderProvider>())
            .and_then(|p| p.downcast_ref::<BlockReaderProvider>())
            .cloned()
            .context("BlockReaderProvider not found")?;

        let db_manager = db_provider.get_manager();
        let block_reader = block_reader_provider.get_reader();
        let cache = context.block_data_cache.clone();

        let state_manager = Arc::new(StateManager::new(db_manager, cache, block_reader));
        self.state_manager = Some(state_manager.clone());

        let reader_provider = StateReaderProvider::new(state_manager);
        drop(providers); // Drop read lock
        context.service_providers.write().unwrap().insert(
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
        let state_manager = self
            .state_manager
            .clone()
            .context("StateManager not initialized")?;

        tokio::spawn(async move {
            let mut last_processed_block = match state_manager.get_last_processed_block() {
                Ok(val) => val,
                Err(e) => {
                    error!("Could not read last processed block: {:?}. Halting.", e);
                    return;
                }
            };

            if let Some(last) = last_processed_block {
                info!("Resuming state processing. Last processed block: {}.", last);
            } else {
                info!("Starting state processing from genesis block 0.");
            }

            let mut rx = context.tx_block_persisted.subscribe();
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        let expected_block = last_processed_block.map_or(0, |n| n + 1);

                        if event.block_number < expected_block {
                            continue;
                        }

                        // If we are behind, trigger the catch-up logic.
                        if event.block_number > expected_block {
                            warn!(
                                "State fell behind. Expected {}, got {}. Attempting to catch up from storage.",
                                expected_block, event.block_number
                            );
                            for b in expected_block..event.block_number {
                                if let Err(e) = state_manager.apply_state_from_storage(b).await {
                                    error!("Failed to apply state for block #{} from storage: {:?}. Halting.", b, e);
                                    return; // Halt on catch-up failure.
                                }
                            }
                        }

                        // Process the current event from the cache.
                        if let Err(e) = state_manager.apply_state_from_block_event(event).await {
                            error!(
                                "Failed to apply state for block event {}: {:?}. Halting.",
                                event.block_number, e
                            );
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
        });
        Ok(())
    }
}
