use crate::state_manager::StateManager;
use anyhow::{Context, Result};
use rock_node_core::{
    database_provider::DatabaseManagerProvider, state_reader::StateReaderProvider, AppContext,
    Plugin,
};
use std::any::TypeId;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{error, info};

/// The main plugin struct that integrates the StateManager into the application.
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

        let db_provider = {
            let providers = context.service_providers.read().unwrap();
            providers
                .get(&TypeId::of::<DatabaseManagerProvider>())
                .and_then(|p| p.downcast_ref::<DatabaseManagerProvider>())
                .cloned()
                .context("DatabaseManagerProvider not found in AppContext")?
        };

        let db_manager = db_provider.get_manager();
        let cache = context.block_data_cache.clone();

        let state_manager = Arc::new(StateManager::new(db_manager, cache));
        self.state_manager = Some(state_manager.clone());

        {
            let provider = StateReaderProvider::new(state_manager);
            let mut providers = context.service_providers.write().unwrap();
            providers.insert(TypeId::of::<StateReaderProvider>(), Arc::new(provider));
        }

        info!("StateReaderProvider registered successfully.");

        self.app_context = Some(context);
        Ok(())
    }

    fn start_internal(&mut self) -> Result<()> {
        info!("Starting State Management Plugin event loop...");
        let context = self
            .app_context
            .clone()
            .context("AppContext not initialized before start")?;
        let state_manager = self
            .state_manager
            .clone()
            .context("StateManager not initialized before start")?;

        tokio::spawn(async move {
            // FIX: The state is now an Option to explicitly handle the uninitialized case.
            let mut last_processed_block: Option<u64> = match state_manager
                .get_last_processed_block()
            {
                Ok(val) => val,
                Err(e) => {
                    error!(
                            "Could not read last processed block from DB: {:?}. Halting state management plugin.",
                            e
                        );
                    return;
                }
            };

            // FIX: More precise logging based on the actual initial state.
            if let Some(last_block) = last_processed_block {
                info!(
                    "State management plugin initialized. Last processed block: {}. Waiting for block {}.",
                    last_block,
                    last_block + 1
                );
            } else {
                info!("State management plugin initialized with fresh state. Waiting for genesis block 0.");
            }

            let mut rx = context.tx_block_persisted.subscribe();

            loop {
                match rx.recv().await {
                    Ok(event) => {
                        // --- REVISED SEQUENTIAL PROCESSING LOGIC ---
                        let expected_block = match last_processed_block {
                            // This is the initial state. We MUST see genesis block 0 first.
                            None => 0,
                            // We are in steady state. We expect the next sequential block.
                            Some(last_block) => last_block + 1,
                        };

                        if event.block_number < expected_block {
                            // This is an old/duplicate event. Ignore it.
                            continue;
                        }

                        if event.block_number > expected_block {
                            // We have missed a block! This is a critical failure for state.
                            error!(
                                "CRITICAL: State consistency violation! Expected block {}, but received {}. Halting processing.",
                                expected_block,
                                event.block_number
                            );
                            break;
                        }

                        // If we reach here, event.block_number == expected_block. Process it.
                        if let Err(e) = state_manager.apply_state_from_block(event) {
                            error!(
                                "Failed to apply state for block {}: {:?}. Halting state management processing.",
                                event.block_number, e
                            );
                            break;
                        }

                        // Update our in-memory state only after successful processing.
                        last_processed_block = Some(event.block_number);
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        error!(
                            "State management plugin lagged by {} messages. State is now out of sync. Shutting down.",
                            n
                        );
                        break;
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        info!("Broadcast channel closed. Shutting down state management plugin event loop.");
                        break;
                    }
                }
            }
        });

        Ok(())
    }
}
