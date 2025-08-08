use crate::metrics::MetricsRegistry;
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio::sync::{broadcast, mpsc};

#[derive(Clone, Debug)]
pub struct AppContext {
    pub config: Arc<super::config::Config>,
    pub metrics: Arc<MetricsRegistry>,
    pub capability_registry: Arc<super::capability::CapabilityRegistry>,
    pub service_providers: Arc<RwLock<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>>,
    pub block_data_cache: Arc<super::cache::BlockDataCache>,

    // Primary pipeline channels
    pub tx_block_items_received: mpsc::Sender<super::events::BlockItemsReceived>,
    pub tx_block_verified: mpsc::Sender<super::events::BlockVerified>,
    pub tx_block_persisted: broadcast::Sender<super::events::BlockPersisted>,

    // New channel for the filtered stream
    pub tx_filtered_block_ready: broadcast::Sender<super::events::FilteredBlockReady>,
}
