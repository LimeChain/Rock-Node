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

    // Can be mpsc as only one system (verifier or persistence) will own the receiver
    pub tx_block_items_received: mpsc::Sender<super::events::BlockItemsReceived>,

    // Can be mpsc as it's a 1-to-1 pipeline step
    pub tx_block_verified: mpsc::Sender<super::events::BlockVerified>,

    // MUST be broadcast as multiple publisher sessions need to know about verification failures
    pub tx_block_verification_failed: broadcast::Sender<super::events::BlockVerificationFailed>,

    // MUST be broadcast to allow multiple concurrent sessions to wait for their specific ACKs
    pub tx_block_persisted: broadcast::Sender<super::events::BlockPersisted>,
}

impl AppContext {
    /// Register a service provider that can be retrieved by type
    pub fn register_service_provider<T: Any + Send + Sync>(&self, provider: Arc<T>) {
        let type_id = TypeId::of::<T>();
        self.service_providers
            .write()
            .unwrap()
            .insert(type_id, provider);
    }

    /// Get a service provider by type
    pub fn get_service_provider<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        let type_id = TypeId::of::<T>();
        self.service_providers
            .read()
            .unwrap()
            .get(&type_id)
            .and_then(|any| any.clone().downcast::<T>().ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::create_isolated_metrics;

    /// Helper function to create a test AppContext
    fn create_test_app_context() -> AppContext {
        let config = Arc::new(super::super::config::Config {
            core: super::super::config::CoreConfig::default(),
            plugins: super::super::config::PluginConfigs::default(),
        });
        let metrics = Arc::new(create_isolated_metrics());
        let capability_registry = Arc::new(super::super::capability::CapabilityRegistry::new());
        let service_providers = Arc::new(RwLock::new(HashMap::new()));
        let block_data_cache = Arc::new(super::super::cache::BlockDataCache::new());

        let (tx_block_items_received, _rx) = mpsc::channel(100);
        let (tx_block_verified, _rx) = mpsc::channel(100);
        let (tx_block_verification_failed, _rx) = broadcast::channel(100);
        let (tx_block_persisted, _rx) = broadcast::channel(100);

        AppContext {
            config,
            metrics,
            capability_registry,
            service_providers,
            block_data_cache,
            tx_block_items_received,
            tx_block_verified,
            tx_block_verification_failed,
            tx_block_persisted,
        }
    }

    #[tokio::test]
    async fn test_app_context_creation() {
        let ctx = create_test_app_context();
        assert_eq!(ctx.config.core.database_path, "data/database");
    }

    #[tokio::test]
    async fn test_app_context_clone() {
        let ctx = create_test_app_context();
        let ctx_clone = ctx.clone();

        // Both should share the same config
        assert_eq!(
            ctx.config.core.database_path,
            ctx_clone.config.core.database_path
        );
    }

    #[tokio::test]
    async fn test_app_context_debug() {
        let ctx = create_test_app_context();
        let debug_str = format!("{:?}", ctx);
        assert!(debug_str.contains("AppContext"));
    }

    #[tokio::test]
    async fn test_block_items_received_channel() {
        let config = Arc::new(super::super::config::Config {
            core: super::super::config::CoreConfig::default(),
            plugins: super::super::config::PluginConfigs::default(),
        });
        let metrics = Arc::new(create_isolated_metrics());
        let capability_registry = Arc::new(super::super::capability::CapabilityRegistry::new());
        let service_providers = Arc::new(RwLock::new(HashMap::new()));
        let block_data_cache = Arc::new(super::super::cache::BlockDataCache::new());

        let (tx_block_items_received, mut rx_block_items_received) = mpsc::channel(100);
        let (tx_block_verified, _rx) = mpsc::channel(100);
        let (tx_block_verification_failed, _rx) = broadcast::channel(100);
        let (tx_block_persisted, _rx) = broadcast::channel(100);

        let ctx = AppContext {
            config,
            metrics,
            capability_registry,
            service_providers,
            block_data_cache,
            tx_block_items_received,
            tx_block_verified,
            tx_block_verification_failed,
            tx_block_persisted,
        };

        let event = super::super::events::BlockItemsReceived {
            block_number: 1,
            cache_key: uuid::Uuid::new_v4(),
        };

        ctx.tx_block_items_received.send(event).await.unwrap();
        let received = rx_block_items_received.recv().await.unwrap();

        assert_eq!(received.block_number, 1);
    }

    #[tokio::test]
    async fn test_block_verified_channel() {
        let config = Arc::new(super::super::config::Config {
            core: super::super::config::CoreConfig::default(),
            plugins: super::super::config::PluginConfigs::default(),
        });
        let metrics = Arc::new(create_isolated_metrics());
        let capability_registry = Arc::new(super::super::capability::CapabilityRegistry::new());
        let service_providers = Arc::new(RwLock::new(HashMap::new()));
        let block_data_cache = Arc::new(super::super::cache::BlockDataCache::new());

        let (tx_block_items_received, _rx) = mpsc::channel(100);
        let (tx_block_verified, mut rx_block_verified) = mpsc::channel(100);
        let (tx_block_verification_failed, _rx) = broadcast::channel(100);
        let (tx_block_persisted, _rx) = broadcast::channel(100);

        let ctx = AppContext {
            config,
            metrics,
            capability_registry,
            service_providers,
            block_data_cache,
            tx_block_items_received,
            tx_block_verified,
            tx_block_verification_failed,
            tx_block_persisted,
        };

        let event = super::super::events::BlockVerified {
            block_number: 2,
            cache_key: uuid::Uuid::new_v4(),
        };

        ctx.tx_block_verified.send(event).await.unwrap();
        let received = rx_block_verified.recv().await.unwrap();

        assert_eq!(received.block_number, 2);
    }

    #[tokio::test]
    async fn test_block_persisted_broadcast_channel() {
        let ctx = create_test_app_context();

        // Create multiple subscribers
        let mut rx1 = ctx.tx_block_persisted.subscribe();
        let mut rx2 = ctx.tx_block_persisted.subscribe();

        let event = super::super::events::BlockPersisted {
            block_number: 3,
            cache_key: uuid::Uuid::new_v4(),
        };

        // Send once
        ctx.tx_block_persisted.send(event).unwrap();

        // Both receivers should get the message
        let received1 = rx1.recv().await.unwrap();
        let received2 = rx2.recv().await.unwrap();

        assert_eq!(received1.block_number, 3);
        assert_eq!(received2.block_number, 3);
        assert_eq!(received1.cache_key, received2.cache_key);
    }

    #[tokio::test]
    async fn test_service_provider_registration() {
        let ctx = create_test_app_context();

        #[derive(Debug)]
        struct TestService {
            value: i32,
        }

        let service = Arc::new(TestService { value: 42 });
        ctx.register_service_provider(service);

        let retrieved = ctx.get_service_provider::<TestService>();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().value, 42);
    }

    #[tokio::test]
    async fn test_service_provider_not_registered() {
        let ctx = create_test_app_context();

        #[derive(Debug)]
        struct UnregisteredService {}

        let retrieved = ctx.get_service_provider::<UnregisteredService>();
        assert!(retrieved.is_none());
    }

    #[tokio::test]
    async fn test_service_provider_multiple_types() {
        let ctx = create_test_app_context();

        #[derive(Debug)]
        struct ServiceA {
            name: String,
        }

        #[derive(Debug)]
        struct ServiceB {
            count: usize,
        }

        let service_a = Arc::new(ServiceA {
            name: "Service A".to_string(),
        });
        let service_b = Arc::new(ServiceB { count: 100 });

        ctx.register_service_provider(service_a);
        ctx.register_service_provider(service_b);

        let retrieved_a = ctx.get_service_provider::<ServiceA>();
        let retrieved_b = ctx.get_service_provider::<ServiceB>();

        assert!(retrieved_a.is_some());
        assert!(retrieved_b.is_some());
        assert_eq!(retrieved_a.unwrap().name, "Service A");
        assert_eq!(retrieved_b.unwrap().count, 100);
    }

    #[tokio::test]
    async fn test_service_provider_overwrite() {
        let ctx = create_test_app_context();

        #[derive(Debug)]
        struct TestService {
            version: u32,
        }

        // Register first version
        ctx.register_service_provider(Arc::new(TestService { version: 1 }));

        // Register second version (overwrites)
        ctx.register_service_provider(Arc::new(TestService { version: 2 }));

        let retrieved = ctx.get_service_provider::<TestService>();
        assert_eq!(retrieved.unwrap().version, 2);
    }

    #[tokio::test]
    async fn test_block_data_cache_access() {
        let ctx = create_test_app_context();
        let block_data = super::super::events::BlockData {
            block_number: 1,
            contents: b"test".to_vec(),
        };

        let key = ctx.block_data_cache.insert(block_data.clone());
        let retrieved = ctx.block_data_cache.get(&key);

        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().block_number, 1);
    }

    #[tokio::test]
    async fn test_capability_registry_access() {
        let ctx = create_test_app_context();

        ctx.capability_registry
            .register(super::super::capability::Capability::ProvidesBlockReader)
            .await;

        let is_registered = ctx
            .capability_registry
            .is_registered(super::super::capability::Capability::ProvidesBlockReader)
            .await;

        assert!(is_registered);
    }

    #[tokio::test]
    async fn test_context_send_sync() {
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}
        assert_send::<AppContext>();
        assert_sync::<AppContext>();
    }

    #[tokio::test]
    async fn test_channels_across_cloned_contexts() {
        let ctx1 = create_test_app_context();
        let ctx2 = ctx1.clone();

        let mut rx = ctx1.tx_block_persisted.subscribe();

        let event = super::super::events::BlockPersisted {
            block_number: 42,
            cache_key: uuid::Uuid::new_v4(),
        };

        // Send from cloned context
        ctx2.tx_block_persisted.send(event).unwrap();

        // Receive from original context's subscription
        let received = rx.recv().await.unwrap();
        assert_eq!(received.block_number, 42);
    }

    #[tokio::test]
    async fn test_service_provider_shared_across_contexts() {
        let ctx1 = create_test_app_context();

        #[derive(Debug)]
        struct SharedService {
            data: std::sync::Mutex<Vec<i32>>,
        }

        let service = Arc::new(SharedService {
            data: std::sync::Mutex::new(vec![1, 2, 3]),
        });

        ctx1.register_service_provider(service);

        // Clone context
        let ctx2 = ctx1.clone();

        // Both contexts should see the same service
        let service1 = ctx1.get_service_provider::<SharedService>().unwrap();
        let service2 = ctx2.get_service_provider::<SharedService>().unwrap();

        service1.data.lock().unwrap().push(4);

        assert_eq!(service2.data.lock().unwrap().len(), 4);
    }

    #[tokio::test]
    async fn test_mpsc_channel_single_receiver() {
        let (tx, mut rx) = mpsc::channel(10);

        let event = super::super::events::BlockItemsReceived {
            block_number: 100,
            cache_key: uuid::Uuid::new_v4(),
        };

        tx.send(event).await.unwrap();

        let received = rx.recv().await.unwrap();
        assert_eq!(received.block_number, 100);
    }

    #[tokio::test]
    async fn test_broadcast_channel_multiple_receivers() {
        let (tx, _) = broadcast::channel::<super::super::events::BlockPersisted>(10);

        let mut rx1 = tx.subscribe();
        let mut rx2 = tx.subscribe();
        let mut rx3 = tx.subscribe();

        let event = super::super::events::BlockPersisted {
            block_number: 200,
            cache_key: uuid::Uuid::new_v4(),
        };

        tx.send(event).unwrap();

        assert_eq!(rx1.recv().await.unwrap().block_number, 200);
        assert_eq!(rx2.recv().await.unwrap().block_number, 200);
        assert_eq!(rx3.recv().await.unwrap().block_number, 200);
    }
}
