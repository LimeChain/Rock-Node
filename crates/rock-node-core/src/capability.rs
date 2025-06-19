use std::collections::HashSet;
use tokio::sync::Mutex;

/// A list of features that can be provided by plugins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    ProvidesVerifiedBlocks,
    ProvidesBlockReader,
}

/// A thread-safe registry where plugins can announce their capabilities.
#[derive(Debug, Default)]
pub struct CapabilityRegistry {
    registered: Mutex<HashSet<Capability>>,
}

impl CapabilityRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn register(&self, capability: Capability) {
        let mut lock = self.registered.lock().await;
        lock.insert(capability);
    }

    pub async fn is_registered(&self, capability: Capability) -> bool {
        let lock = self.registered.lock().await;
        lock.contains(&capability)
    }
}
