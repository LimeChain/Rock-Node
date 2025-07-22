mod cold_storage;
mod hot_tier;
mod service;
mod state;

use crate::service::PersistenceService;
use async_trait::async_trait;
use prost::Message;
use rock_node_core::{
    app_context::AppContext,
    block_reader::BlockReader,
    block_writer::{BlockWriter, BlockWriterProvider},
    capability::Capability,
    database_provider::DatabaseManagerProvider,
    error::{Error as CoreError, Result as CoreResult},
    events::{BlockItemsReceived, BlockPersisted, BlockVerified},
    plugin::Plugin,
    BlockReaderProvider,
};
use rock_node_protobufs::com::hedera::hapi::block::stream::Block;
use std::{
    any::TypeId,
    cmp::min,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    sync::{mpsc::Receiver, Notify},
    time::sleep,
};
use tracing::{debug, error, info, trace, warn};
use uuid::Uuid;

enum InboundEvent {
    Verified(BlockVerified),
    Unverified(BlockItemsReceived),
}
impl InboundEvent {
    fn block_number(&self) -> u64 {
        match self {
            InboundEvent::Verified(e) => e.block_number,
            InboundEvent::Unverified(e) => e.block_number,
        }
    }
    fn cache_key(&self) -> Uuid {
        match self {
            InboundEvent::Verified(e) => e.cache_key,
            InboundEvent::Unverified(e) => e.cache_key,
        }
    }
}

pub struct PersistencePlugin {
    context: Option<AppContext>,
    rx_block_items_received: Option<Receiver<BlockItemsReceived>>,
    rx_block_verified: Option<Receiver<BlockVerified>>,
    service: Option<PersistenceService>,
    running: Arc<AtomicBool>,
    shutdown_notify: Arc<Notify>,
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
            service: None,
            running: Arc::new(AtomicBool::new(false)),
            shutdown_notify: Arc::new(Notify::new()),
        }
    }
}

#[async_trait]
impl Plugin for PersistencePlugin {
    fn name(&self) -> &'static str {
        "persistence-plugin"
    }

    fn initialize(&mut self, context: AppContext) -> CoreResult<()> {
        info!("Initializing new tiered PersistencePlugin...");

        let db_provider = context
            .service_providers
            .read()
            .map_err(|e| {
                CoreError::PluginInitialization(format!("Failed to read service_providers: {}", e))
            })?
            .get(&TypeId::of::<DatabaseManagerProvider>())
            .and_then(|any| any.downcast_ref::<DatabaseManagerProvider>().cloned())
            .ok_or_else(|| {
                CoreError::PluginInitialization("DatabaseManagerProvider not found!".to_string())
            })?;

        let db_manager = db_provider.get_manager();
        let db_handle = db_manager.db_handle();
        let config_arc = Arc::new(context.config.plugins.persistence_service.clone());
        let metrics_arc = context.metrics.clone();

        let state_manager = Arc::new(state::StateManager::new(db_handle.clone()));
        let hot_tier = Arc::new(hot_tier::HotTier::new(db_handle.clone()));
        let cold_writer = Arc::new(cold_storage::writer::ColdWriter::new(config_arc.clone()));

        debug!("Building cold storage index...");
        let cold_reader = Arc::new(cold_storage::reader::ColdReader::new(
            config_arc.clone(),
            metrics_arc.clone(),
        ));
        cold_reader.scan_and_build_index()?;

        if state_manager.get_true_earliest_persisted()?.is_none() {
            debug!("Determining true earliest block number for the first time...");
            let earliest_cold = cold_reader.get_earliest_indexed_block()?;
            let earliest_hot = hot_tier.get_earliest_block_number()?;

            let true_earliest = match (earliest_cold, earliest_hot) {
                (Some(c), Some(h)) => Some(min(c, h)),
                (Some(c), None) => Some(c),
                (None, Some(h)) => Some(h),
                (None, None) => None,
            };

            if let Some(earliest) = true_earliest {
                state_manager.initialize_true_earliest_persisted(earliest)?;
                info!("Set true earliest persisted block to #{}", earliest);
            } else {
                info!("No existing blocks found in hot or cold storage.");
            }
        }

        let archiver = Arc::new(cold_storage::archiver::Archiver::new(
            config_arc,
            hot_tier.clone(),
            cold_writer,
            state_manager.clone(),
            cold_reader.clone(),
            metrics_arc.clone(),
            self.shutdown_notify.clone(),
        ));

        let service = service::PersistenceService::new(
            hot_tier,
            cold_reader,
            archiver.clone(),
            state_manager,
            metrics_arc,
        );
        let service_arc = Arc::new(service.clone());
        self.service = Some(service);

        // Spawn the dedicated background archiver task.
        tokio::spawn(async move {
            info!("Starting background Archiver task.");
            let archival_check_interval = Duration::from_secs(30);

            loop {
                tokio::select! {
                    _ = archiver.shutdown_notify.notified() => {
                        info!("Archiver received shutdown signal. Exiting loop.");
                        break;
                    }
                    _ = archiver.trigger.notified() => {
                        trace!("Archiver triggered by write notification.");
                    }
                    _ = sleep(archival_check_interval) => {
                        trace!("Archiver triggered by periodic 30s check.");
                    }
                }

                if let Err(e) = archiver.run_archival_cycle() {
                    warn!("Error during background archival cycle: {}", e);
                }
            }
            info!("Archiver task has terminated.");
        });

        self.context = Some(context.clone());

        {
            let mut providers = context.service_providers.write().map_err(|_| {
                CoreError::PluginInitialization(
                    "Failed to acquire write lock on service providers".to_string(),
                )
            })?;
            let reader_provider =
                BlockReaderProvider::new(service_arc.clone() as Arc<dyn BlockReader>);
            providers.insert(
                TypeId::of::<BlockReaderProvider>(),
                Arc::new(reader_provider),
            );
            let writer_provider = BlockWriterProvider::new(service_arc as Arc<dyn BlockWriter>);
            providers.insert(
                TypeId::of::<BlockWriterProvider>(),
                Arc::new(writer_provider),
            );
            debug!("PersistencePlugin registered providers for BlockReader and BlockWriter.");
        }
        Ok(())
    }

    fn start(&mut self) -> CoreResult<()> {
        info!("Starting Persistence Plugin event loop...");
        let context = self.context.as_ref().cloned().ok_or_else(|| {
            CoreError::PluginInitialization("PersistencePlugin not initialized".to_string())
        })?;
        let service = self.service.as_ref().cloned().ok_or_else(|| {
            CoreError::PluginInitialization("PersistenceService not initialized".to_string())
        })?;
        let mut rx_verified = self.rx_block_verified.take().ok_or_else(|| {
            CoreError::PluginInitialization("Block verified receiver not set".to_string())
        })?;
        let mut rx_items_received_opt = self.rx_block_items_received.take();

        let shutdown_notify = self.shutdown_notify.clone();
        let running_clone = self.running.clone();

        self.running.store(true, Ordering::SeqCst);
        tokio::spawn(async move {
            let use_verified_stream = context
                .capability_registry
                .is_registered(Capability::ProvidesVerifiedBlocks)
                .await;

            loop {
                let event_future = async {
                    if use_verified_stream {
                        rx_verified.recv().await.map(InboundEvent::Verified)
                    } else if let Some(rx) = &mut rx_items_received_opt {
                        rx.recv().await.map(InboundEvent::Unverified)
                    } else {
                        None
                    }
                };

                tokio::select! {
                    _ = shutdown_notify.notified() => {
                        info!("PersistencePlugin event loop received shutdown signal. Exiting.");
                        break;
                    }
                    event_opt = event_future => {
                        if let Some(event) = event_opt {
                            process_event(event, &context, &service).await;
                        } else {
                            // A channel closed, which is a natural end to the loop.
                            info!("Upstream channel closed. PersistencePlugin event loop is terminating.");
                            break;
                        }
                    }
                }
            }
            running_clone.store(false, Ordering::SeqCst);
            info!("PersistencePlugin event loop has terminated.");
        });
        Ok(())
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    async fn stop(&mut self) -> CoreResult<()> {
        info!("Stopping PersistencePlugin...");
        self.shutdown_notify.notify_waiters();
        self.running.store(false, Ordering::SeqCst);
        Ok(())
    }
}

async fn process_event(event: InboundEvent, context: &AppContext, service: &PersistenceService) {
    let block_number = event.block_number();
    let cache_key = event.cache_key();
    if let Some(data) = context.block_data_cache.get(&cache_key) {
        match Block::decode(data.contents.as_slice()) {
            Ok(block_proto) => {
                if let Err(e) = service.write_block(&block_proto) {
                    error!("Failed to persist block #{}: {}", block_number, e);
                } else {
                    trace!("Successfully persisted block #{}.", block_number);
                    let persisted_event = BlockPersisted {
                        block_number,
                        cache_key,
                    };
                    if context.tx_block_persisted.send(persisted_event).is_err() {
                        warn!("No active subscribers for persisted block events.");
                    }
                }
            }
            Err(e) => {
                warn!(
                    "Could not decode BlockData from cache for block #{}: {}",
                    block_number, e
                )
            }
        }
    } else {
        warn!(
            "Could not find data for block #{} in cache with key [{}].",
            block_number, cache_key
        );
    }
    // Still mark for removal even if processing failed, to prevent cache leaks.
    context.block_data_cache.mark_for_removal(cache_key).await;
}
