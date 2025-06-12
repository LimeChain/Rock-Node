use crate::capability::CapabilityRegistry;
use crate::config::Config;
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// The shared context that is passed to all plugins.
/// It holds handles to all core, shared facilities of the application.
/// It is cloneable and thread-safe.
#[derive(Clone, Debug)]
pub struct AppContext {
    pub config: Arc<Config>,
    pub capability_registry: Arc<CapabilityRegistry>,
    /// A type-erased service locator for plugins to provide `Trait` implementations
    /// to other plugins without direct coupling.
    pub service_providers: Arc<RwLock<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>>,
} 