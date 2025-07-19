mod cold_storage;
mod hot_tier;
mod service;
mod state;

use crate::service::PersistenceService;
use anyhow::anyhow;
use prost::Message;
use rock_node_core::{
    app_context::AppContext,
    block_reader::BlockReader,
    block_writer::BlockWriter,
    capability::Capability,
    database_provider::DatabaseManagerProvider,
    error::{Error as CoreError, Result as CoreResult},
    events::{BlockItemsReceived, BlockPersisted, BlockVerified},
    plugin::Plugin,
    BlockReaderProvider,
};
use rock_node_protobufs::com::hedera::hapi::block::stream::Block;
use std::{any::TypeId, cmp::min, sync::Arc, time::Duration};
use tokio::{sync::mpsc::Receiver, time::sleep};
use tracing::{debug, error, info, trace, warn};
use uuid::Uuid;

enum InboundEvent {
    Verified(BlockVerified),
    Unverified(BlockItemsReceived),
}
impl InboundEvent {
    fn block_number(&self) -> u64 {
        self.into()
    }
    fn cache_key(&self) -> Uuid {
        self.into()
    }
}
impl From<&InboundEvent> for u64 {
    fn from(event: &InboundEvent) -> Self {
        match event {
            InboundEvent::Verified(e) => e.block_number,
            InboundEvent::Unverified(e) => e.block_number,
        }
    }
}
impl From<&InboundEvent> for Uuid {
    fn from(event: &InboundEvent) -> Self {
        match event {
            InboundEvent::Verified(e) => e.cache_key,
            InboundEvent::Unverified(e) => e.cache_key,
        }
    }
}

#[derive(Clone)]
pub struct BlockWriterProvider {
    writer: Arc<dyn BlockWriter>,
}
impl BlockWriterProvider {
    pub fn new(writer: Arc<dyn BlockWriter>) -> Self {
        Self { writer }
    }
    pub fn get_writer(&self) -> Arc<dyn BlockWriter> {
        self.writer.clone()
    }
}

pub struct PersistencePlugin {
    context: Option<AppContext>,
    rx_block_items_received: Option<Receiver<BlockItemsReceived>>,
    rx_block_verified: Option<Receiver<BlockVerified>>,
    service: Option<PersistenceService>,
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
        }
    }
}

impl Plugin for PersistencePlugin {
    fn name(&self) -> &'static str {
        "persistence-plugin"
    }

    fn initialize(&mut self, context: AppContext) -> CoreResult<()> {
        info!("Initializing new tiered PersistencePlugin...");

        let db_provider = context
            .service_providers
            .read()
            .unwrap()
            .get(&TypeId::of::<DatabaseManagerProvider>())
            .and_then(|any| any.downcast_ref::<DatabaseManagerProvider>().cloned())
            .ok_or_else(|| anyhow!("DatabaseManagerProvider not found!"))?;

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
            // As per the design doc, a periodic check is recommended.
            let archival_check_interval = Duration::from_secs(30);

            loop {
                tokio::select! {
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
        });

        self.context = Some(context.clone());

        {
            let mut providers = context
                .service_providers
                .write()
                .map_err(|_| anyhow!("Failed to acquire write lock on service providers"))?;
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
        let context = self
            .context
            .as_ref()
            .ok_or_else(|| {
                CoreError::PluginInitialization("PersistencePlugin not initialized".to_string())
            })?
            .clone();
        let service = self
            .service
            .as_ref()
            .ok_or_else(|| {
                CoreError::PluginInitialization("PersistenceService not initialized".to_string())
            })?
            .clone();
        let mut rx_verified = self.rx_block_verified.take().ok_or_else(|| {
            CoreError::PluginInitialization("Block verified receiver not set".to_string())
        })?;
        let rx_items_received_opt = self.rx_block_items_received.take();
        tokio::spawn(async move {
            let use_verified_stream = context
                .capability_registry
                .is_registered(Capability::ProvidesVerifiedBlocks)
                .await;
            if use_verified_stream {
                debug!("Subscribing to 'BlockVerified' events.");
                while let Some(event) = rx_verified.recv().await {
                    process_event(InboundEvent::Verified(event), &context, &service).await;
                }
            } else {
                debug!("Subscribing to 'BlockItemsReceived' events.");
                if let Some(mut rx_items) = rx_items_received_opt {
                    while let Some(event) = rx_items.recv().await {
                        process_event(InboundEvent::Unverified(event), &context, &service).await;
                    }
                } else {
                    error!(
                        "FATAL: PersistencePlugin misconfigured. No verifier and no unverified receiver. \
                        The persistence pipeline cannot function without an event source."
                    );
                    // Exit the event loop - the plugin is non-functional
                    return;
                }
            }
        });
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
                    let _ = context.tx_block_persisted.send(persisted_event);
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
    context.block_data_cache.mark_for_removal(cache_key).await;
}
