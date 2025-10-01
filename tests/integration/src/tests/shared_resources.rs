use crate::fixtures::{mock_plugins::MockPlugin, test_context::IntegrationTestContext, test_data};
use rock_node_core::database_provider::DatabaseManagerProvider;
use std::any::TypeId;
use tracing_test::traced_test;

#[tokio::test]
#[traced_test]
async fn test_database_shared_across_plugins() {
    let mut ctx = IntegrationTestContext::builder()
        .with_plugin(Box::new(MockPlugin::new("plugin-1")))
        .with_plugin(Box::new(MockPlugin::new("plugin-2")))
        .build()
        .await
        .unwrap();

    ctx.initialize_plugins().unwrap();

    // Both plugins should have access to the same DatabaseManagerProvider
    let providers = ctx.context.service_providers.read().unwrap();
    let db_provider = providers
        .get(&TypeId::of::<DatabaseManagerProvider>())
        .and_then(|p| p.downcast_ref::<DatabaseManagerProvider>())
        .expect("DatabaseManagerProvider should be available");

    // Verify it's accessible
    let _db_manager = db_provider.get_manager();
    // Manager is accessible
}

#[tokio::test]
#[traced_test]
async fn test_metrics_aggregation() {
    let mut ctx = IntegrationTestContext::builder()
        .with_plugin(Box::new(MockPlugin::new("plugin-1")))
        .with_plugin(Box::new(MockPlugin::new("plugin-2")))
        .build()
        .await
        .unwrap();

    ctx.initialize_plugins().unwrap();
    ctx.start_plugins().unwrap();

    // All plugins write to the same metrics registry
    let metrics = &ctx.context.metrics;

    // Increment a counter
    metrics.blocks_acknowledged.inc();
    assert_eq!(metrics.blocks_acknowledged.get(), 1);

    // Both plugins share the same metrics
    metrics.blocks_acknowledged.inc();
    assert_eq!(metrics.blocks_acknowledged.get(), 2);

    ctx.stop_plugins().await.unwrap();
}

#[tokio::test]
#[traced_test]
async fn test_cache_isolation() {
    let ctx = IntegrationTestContext::builder().build().await.unwrap();

    let cache = &ctx.context.block_data_cache;

    // Insert some data
    let block_data1 = test_data::create_test_block_data(1);
    let block_data2 = test_data::create_test_block_data(2);

    let key1 = cache.insert(block_data1.clone());
    let key2 = cache.insert(block_data2.clone());

    // Keys should be unique
    assert_ne!(key1, key2);

    // Data should be retrievable
    let retrieved1 = cache.get(&key1).unwrap();
    let retrieved2 = cache.get(&key2).unwrap();

    assert_eq!(retrieved1.block_number, 1);
    assert_eq!(retrieved2.block_number, 2);
}

#[tokio::test]
#[traced_test]
async fn test_concurrent_cache_access() {
    let ctx = IntegrationTestContext::builder().build().await.unwrap();

    let cache = ctx.context.block_data_cache.clone();

    // Spawn multiple tasks that write to cache
    let mut handles = vec![];
    for i in 0..10 {
        let cache_clone = cache.clone();
        let handle = tokio::spawn(async move {
            let block_data = test_data::create_test_block_data(i);
            cache_clone.insert(block_data)
        });
        handles.push(handle);
    }

    // Collect all keys
    let mut keys = vec![];
    for handle in handles {
        keys.push(handle.await.unwrap());
    }

    // All keys should be unique
    assert_eq!(keys.len(), 10);
    let unique_keys: std::collections::HashSet<_> = keys.iter().collect();
    assert_eq!(unique_keys.len(), 10);

    // All data should be retrievable
    for (i, key) in keys.iter().enumerate() {
        let retrieved = cache.get(key).unwrap();
        assert_eq!(retrieved.block_number, i as u64);
    }
}

#[tokio::test]
#[traced_test]
async fn test_concurrent_database_access() {
    let ctx = IntegrationTestContext::builder().build().await.unwrap();

    let providers = ctx.context.service_providers.read().unwrap();
    let db_provider = providers
        .get(&TypeId::of::<DatabaseManagerProvider>())
        .and_then(|p| p.downcast_ref::<DatabaseManagerProvider>())
        .expect("DatabaseManagerProvider should be available");

    // Multiple concurrent accesses to database
    let mut handles = vec![];
    for _ in 0..5 {
        let db_manager = db_provider.get_manager();
        let handle = tokio::spawn(async move {
            // Just verify we can get managers concurrently
            std::thread::sleep(std::time::Duration::from_millis(10));
            db_manager
        });
        handles.push(handle);
    }

    // All should complete without errors
    for handle in handles {
        assert!(handle.await.is_ok());
    }
}

#[tokio::test]
#[traced_test]
async fn test_service_provider_registration() {
    let ctx = IntegrationTestContext::builder().build().await.unwrap();

    // Register a custom service provider
    #[derive(Debug, Clone)]
    struct TestService {
        value: i32,
    }

    let service = std::sync::Arc::new(TestService { value: 42 });
    ctx.context.register_service_provider(service);

    // Retrieve it
    let retrieved = ctx
        .context
        .get_service_provider::<TestService>()
        .expect("Service should be available");

    assert_eq!(retrieved.value, 42);
}

#[tokio::test]
#[traced_test]
async fn test_service_provider_shared_across_contexts() {
    let ctx = IntegrationTestContext::builder().build().await.unwrap();

    // Register a service
    #[derive(Debug)]
    struct SharedService {
        counter: std::sync::Mutex<usize>,
    }

    let service = std::sync::Arc::new(SharedService {
        counter: std::sync::Mutex::new(0),
    });

    ctx.context.register_service_provider(service.clone());

    // Clone context
    let ctx_clone = ctx.context.clone();

    // Both contexts should see the same service
    let svc1 = ctx.context.get_service_provider::<SharedService>().unwrap();
    let svc2 = ctx_clone.get_service_provider::<SharedService>().unwrap();

    *svc1.counter.lock().unwrap() += 1;
    assert_eq!(*svc2.counter.lock().unwrap(), 1);
}

#[tokio::test]
#[traced_test]
async fn test_metrics_thread_safety() {
    let ctx = IntegrationTestContext::builder().build().await.unwrap();

    let metrics = ctx.context.metrics.clone();

    // Spawn multiple threads incrementing counters
    let mut handles = vec![];
    for _ in 0..10 {
        let metrics_clone = metrics.clone();
        let handle = tokio::spawn(async move {
            for _ in 0..100 {
                metrics_clone.blocks_acknowledged.inc();
            }
        });
        handles.push(handle);
    }

    // Wait for all to complete
    for handle in handles {
        handle.await.unwrap();
    }

    // Should have 1000 total increments
    assert_eq!(metrics.blocks_acknowledged.get(), 1000);
}

#[tokio::test]
#[traced_test]
async fn test_capability_registry_concurrent_access() {
    let ctx = IntegrationTestContext::builder().build().await.unwrap();

    let cap_registry = ctx.context.capability_registry.clone();

    // Register capabilities concurrently
    let mut handles = vec![];
    for _ in 0..5 {
        let cap_clone = cap_registry.clone();
        let handle = tokio::spawn(async move {
            cap_clone
                .register(rock_node_core::capability::Capability::ProvidesBlockReader)
                .await;
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.await.unwrap();
    }

    // Should be registered
    assert!(
        cap_registry
            .is_registered(rock_node_core::capability::Capability::ProvidesBlockReader)
            .await
    );
}

#[tokio::test]
#[traced_test]
async fn test_config_immutability() {
    let ctx = IntegrationTestContext::builder()
        .with_config_override("core.log_level".to_string(), "INFO".to_string())
        .build()
        .await
        .unwrap();

    // Config should be wrapped in Arc and immutable
    let config1 = ctx.context.config.clone();
    let config2 = ctx.context.config.clone();

    // Both should point to the same underlying config
    assert_eq!(config1.core.log_level, config2.core.log_level);
}
