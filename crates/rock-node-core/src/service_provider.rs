// In crates/rock-node-core/src/service_provider.rs

use crate::block_reader::BlockReader;
use std::sync::Arc;

/// A concrete, shareable handle for the BlockReader service.
///
/// Because this is a concrete struct, it can be safely stored in the
/// `AppContext`'s service map as `Arc<dyn Any + Send + Sync>` and
/// reliably downcast back to `Arc<BlockReaderProvider>`.
pub struct BlockReaderProvider {
    // This field is private to enforce decoupling. The consumer can only
    // get the service via the `get_service` method.
    service: Arc<dyn BlockReader>,
}

impl BlockReaderProvider {
    /// Creates a new provider handle containing the service.
    pub fn new(service: Arc<dyn BlockReader>) -> Self {
        Self { service }
    }

    /// Allows a consumer to retrieve a clone of the service Arc.
    pub fn get_service(&self) -> Arc<dyn BlockReader> {
        self.service.clone()
    }
}
