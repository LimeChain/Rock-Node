mod storage;

use crate::storage::StorageManager;
use rock_node_core::{
    app_context::AppContext,
    capability::Capability,
    error::Result,
    events::{BlockItemsReceived, BlockPersisted, BlockVerified},
    plugin::Plugin,
    block_reader::BlockReader,
};
use std::{any::TypeId, sync::Arc};
use tokio::sync::mpsc::Receiver;
use tracing::{info, warn};

/// A generic event wrapper to allow one processing function
#[derive(Debug)]
enum InboundEvent {
    Verified(BlockVerified),
    Unverified(BlockItemsReceived),
}

impl InboundEvent {
    fn block_number(&self) -> u64 {
        match self {
            Self::Verified(e) => e.block_number,
            Self::Unverified(e) => e.block_number,
        }
    }
    fn cache_key(&self) -> uuid::Uuid {
        match self {
            Self::Verified(e) => e.cache_key,
            Self::Unverified(e) => e.cache_key,
        }
    }
}
pub struct PersistencePlugin {
    context: Option<AppContext>,
    rx_block_items_received: Option<Receiver<BlockItemsReceived>>,
    rx_block_verified: Option<Receiver<BlockVerified>>,
    storage_manager: Option<StorageManager>,
}

impl PersistencePlugin {
    pub fn new(
        rx_block_items_received: Option<Receiver<BlockItemsReceived>>,
        rx_block_verified: Receiver<BlockVerified>,
    ) -> Self {
        Self {
            context: None,
            rx_block_items_received,
            rx_block_verified: Some(rx_block_verified),
            storage_manager: None,
        }
    }
}

impl Plugin for PersistencePlugin {
    fn name(&self) -> &'static str { "persistence-plugin" }

    fn initialize(&mut self, context: AppContext) -> Result<()> {
        info!("Initializing PersistencePlugin...");
        let config = &context.config.plugins.persistence_service;
        let storage_manager =
            StorageManager::new(&config.storage_path, config.hot_storage_block_count)?;
        info!(
            "StorageManager initialized at path '{}' with hot tier limit of {} blocks.",
            &config.storage_path, config.hot_storage_block_count
        );
        self.storage_manager = Some(storage_manager.clone());
        let storage_manager_arc = Arc::new(storage_manager);

        // --- FIX: Perform all borrowing operations within a dedicated scope ---
        {
            // The borrow starts here.
            let mut providers = context.service_providers.write().unwrap();
            providers.insert(TypeId::of::<Arc<dyn BlockReader>>(), storage_manager_arc);
            info!("PersistencePlugin registered as a BlockReader provider.");
        } // -- The borrow (the `providers` lock guard) ends here automatically.

        // Now that the borrow is released, it's safe to move `context`.
        self.context = Some(context);
        
        Ok(())
    }

    fn start(&mut self) -> Result<()> {
        info!("Starting PersistencePlugin...");
        let context = self.context.as_ref().expect("Plugin must be initialized").clone();
        let storage_manager = self
            .storage_manager
            .as_ref()
            .expect("Plugin must be initialized")
            .clone();

        let mut rx_verified = self.rx_block_verified.take().unwrap();
        let rx_items_received_opt = self.rx_block_items_received.take();

        tokio::spawn(async move {
            let use_verified_stream = context
                .capability_registry
                .is_registered(Capability::ProvidesVerifiedBlocks)
                .await;

            if use_verified_stream {
                info!("Verifier plugin detected. Subscribing to 'BlockVerified' events.");
                while let Some(event) = rx_verified.recv().await {
                    process_event(InboundEvent::Verified(event), &context, &storage_manager).await;
                }
            } else {
                info!("No verifier plugin detected. Subscribing to 'BlockItemsReceived' events.");
                if let Some(mut rx_items) = rx_items_received_opt {
                    while let Some(event) = rx_items.recv().await {
                        process_event(InboundEvent::Unverified(event), &context, &storage_manager)
                            .await;
                    }
                } else {
                    panic!("FATAL: PersistencePlugin is configured to listen for unverified items, but was not given a receiver.");
                }
            }
        });

        Ok(())
    }
}

// process_event function is correct and does not need changes.
async fn process_event(event: InboundEvent, context: &AppContext, storage_manager: &StorageManager) {
    let block_number = event.block_number();
    let cache_key = event.cache_key();
    info!(
        "Persistence: Processing block #{} with cache key [{}].",
        block_number, cache_key
    );
    if let Some(data) = context.block_data_cache.get(&cache_key) {
        if let Err(e) = storage_manager.write_block(block_number, &data) {
            warn!("CRITICAL: Failed to persist block #{}: {}", block_number, e);
        } else {
            info!("Persistence: Successfully wrote block #{} to storage.", block_number);
            let persisted_event = BlockPersisted {
                block_number,
                cache_key,
            };
            if context.tx_block_persisted.send(persisted_event).is_err() {
                warn!(
                    "Failed to publish BlockPersisted event for block #{}. All subscribers have disconnected.",
                    block_number
                );
            }
        }
    } else {
        warn!(
            "Could not find data for block #{} in cache with key [{}]. It may have expired or been processed by another instance.",
            block_number, cache_key
        );
    }
    context.block_data_cache.remove(&cache_key);
}
