use anyhow::Result;
use std::fmt::Debug;
use std::sync::Arc;

/// The public, read-only interface for accessing the live ledger state.
///
/// This trait is implemented by the State Management Plugin and can be
/// consumed by other plugins (e.g., ProofService, QueryPlugin) to retrieve
/// state data without being coupled to the underlying implementation.
pub trait StateReader: Debug + Send + Sync + 'static {
    /// Retrieves the raw value for a given state key.
    ///
    /// The key is a composite key derived from the `state_id` and the
    /// entity-specific key defined in the protobufs.
    ///
    /// # Arguments
    /// * `key` - The byte slice representing the full state key.
    ///
    /// # Returns
    /// A `Result` containing an `Option<Vec<u8>>`.
    /// - `Ok(Some(value))` if the key exists. The value is the serialized protobuf message.
    /// - `Ok(None)` if the key does not exist.
    /// - `Err(e)` if a database error occurs.
    fn get_state_value(&self, key: &[u8]) -> Result<Option<Vec<u8>>>;

    // In the future, this trait can be extended to support more complex queries
    // or to generate Merkle proofs directly.
    //
    // fn get_merkle_proof(&self, key: &[u8]) -> Result<MerkleProof>;
}

/// A concrete, shareable handle for the StateReader service.
///
/// This provider is registered in the `AppContext` by the State Plugin at startup.
/// Other plugins can then request this provider to get access to the `StateReader` service.
#[derive(Clone)]
pub struct StateReaderProvider {
    reader: Arc<dyn StateReader>,
}

impl StateReaderProvider {
    /// Creates a new provider handle containing the service.
    pub fn new(reader: Arc<dyn StateReader>) -> Self {
        Self { reader }
    }

    /// Allows a consumer to retrieve a clone of the service `Arc`.
    pub fn get_reader(&self) -> Arc<dyn StateReader> {
        self.reader.clone()
    }
}
