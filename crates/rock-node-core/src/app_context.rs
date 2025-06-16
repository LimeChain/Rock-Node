use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio::sync::{broadcast, mpsc};

#[derive(Clone, Debug)]
pub struct AppContext {
    pub config: Arc<super::config::Config>,
    pub capability_registry: Arc<super::capability::CapabilityRegistry>,
    pub service_providers: Arc<RwLock<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>>,
    pub block_data_cache: Arc<super::cache::BlockDataCache>,
    
    // Can be mpsc as only one system (verifier or persistence) will own the receiver
    pub tx_block_items_received: mpsc::Sender<super::events::BlockItemsReceived>,
    
    // Can be mpsc as it's a 1-to-1 pipeline step
    pub tx_block_verified: mpsc::Sender<super::events::BlockVerified>,

    // MUST be broadcast to allow multiple concurrent sessions to wait for their specific ACKs
    pub tx_block_persisted: broadcast::Sender<super::events::BlockPersisted>,
}