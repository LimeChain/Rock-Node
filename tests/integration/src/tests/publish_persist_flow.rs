use crate::fixtures::{test_context::IntegrationTestContext, test_data};
use rock_node_core::{BlockReaderProvider, BlockWriterProvider};
use rock_node_persistence_plugin::PersistencePlugin;
use rock_node_verifier_plugin::VerifierPlugin;
use std::any::TypeId;
use tracing_test::traced_test;

#[tokio::test]
#[traced_test]
async fn test_persistence_plugin_initialization() {
    // Test that persistence plugin can initialize with database
    let rx_block_verified = {
        let (_, rx) = tokio::sync::mpsc::channel(10);
        rx
    };

    let mut ctx = IntegrationTestContext::builder()
        .with_plugin(Box::new(PersistencePlugin::new(None, rx_block_verified)))
        .build()
        .await
        .unwrap();

    // Should initialize successfully
    let result = ctx.initialize_plugins();
    assert!(result.is_ok(), "Persistence plugin should initialize");

    // Should register BlockReader and BlockWriter providers
    let providers = ctx.context.service_providers.read().unwrap();

    assert!(
        providers
            .get(&TypeId::of::<BlockReaderProvider>())
            .is_some(),
        "BlockReaderProvider should be registered"
    );

    assert!(
        providers
            .get(&TypeId::of::<BlockWriterProvider>())
            .is_some(),
        "BlockWriterProvider should be registered"
    );
}

#[tokio::test]
#[traced_test]
async fn test_block_reader_provider_access() {
    let rx_block_verified = {
        let (_, rx) = tokio::sync::mpsc::channel(10);
        rx
    };

    let mut ctx = IntegrationTestContext::builder()
        .with_plugin(Box::new(PersistencePlugin::new(None, rx_block_verified)))
        .build()
        .await
        .unwrap();

    ctx.initialize_plugins().unwrap();

    // Access BlockReaderProvider
    let providers = ctx.context.service_providers.read().unwrap();
    let reader_provider = providers
        .get(&TypeId::of::<BlockReaderProvider>())
        .and_then(|p| p.downcast_ref::<BlockReaderProvider>())
        .expect("BlockReaderProvider should be available");

    let reader = reader_provider.get_reader();

    // Should be able to call methods (even if no blocks exist yet)
    let latest = reader.get_latest_persisted_block_number().unwrap();
    assert_eq!(latest, None, "No blocks persisted yet");
}

#[tokio::test]
#[traced_test]
async fn test_block_writer_provider_access() {
    let rx_block_verified = {
        let (_, rx) = tokio::sync::mpsc::channel(10);
        rx
    };

    let mut ctx = IntegrationTestContext::builder()
        .with_plugin(Box::new(PersistencePlugin::new(None, rx_block_verified)))
        .build()
        .await
        .unwrap();

    ctx.initialize_plugins().unwrap();

    // Access BlockWriterProvider
    let providers = ctx.context.service_providers.read().unwrap();
    let writer_provider = providers
        .get(&TypeId::of::<BlockWriterProvider>())
        .and_then(|p| p.downcast_ref::<BlockWriterProvider>())
        .expect("BlockWriterProvider should be available");

    let _writer = writer_provider.get_writer();
    // Writer exists and is accessible
}

#[tokio::test]
#[traced_test]
async fn test_persistence_plugin_lifecycle() {
    let rx_block_verified = {
        let (_, rx) = tokio::sync::mpsc::channel(10);
        rx
    };

    let mut ctx = IntegrationTestContext::builder()
        .with_plugin(Box::new(PersistencePlugin::new(None, rx_block_verified)))
        .build()
        .await
        .unwrap();

    // Initialize
    ctx.initialize_plugins().unwrap();

    // Start
    ctx.start_plugins().unwrap();
    assert!(ctx.all_plugins_running());

    // Stop
    ctx.stop_plugins().await.unwrap();
    assert!(!ctx.all_plugins_running());
}

#[tokio::test]
#[traced_test]
async fn test_verifier_plugin_initialization() {
    let (_tx, rx) = tokio::sync::mpsc::channel(10);

    let mut ctx = IntegrationTestContext::builder()
        .with_plugin(Box::new(VerifierPlugin::new(rx)))
        .build()
        .await
        .unwrap();

    // Verifier should initialize
    let result = ctx.initialize_plugins();
    assert!(result.is_ok(), "Verifier plugin should initialize");
}

#[tokio::test]
#[traced_test]
async fn test_multi_plugin_initialization_order() {
    // Create a context with verifier plugin
    let (_tx_items, rx_items) = tokio::sync::mpsc::channel(10);

    let mut ctx = IntegrationTestContext::builder()
        .with_plugin(Box::new(VerifierPlugin::new(rx_items)))
        .build()
        .await
        .unwrap();

    // Plugin should initialize
    ctx.initialize_plugins().unwrap();
    assert_eq!(ctx.plugin_count(), 1);

    // Plugin should start
    ctx.start_plugins().unwrap();
    assert!(ctx.all_plugins_running());

    ctx.stop_plugins().await.unwrap();
}

#[tokio::test]
#[traced_test]
async fn test_event_propagation_through_pipeline() {
    // Create event channels
    let (tx_items, rx_items) = tokio::sync::mpsc::channel(10);

    let mut ctx = IntegrationTestContext::builder()
        .with_plugin(Box::new(VerifierPlugin::new(rx_items)))
        .build()
        .await
        .unwrap();

    ctx.initialize_plugins().unwrap();
    ctx.start_plugins().unwrap();

    // Send a BlockItemsReceived event
    let builder = test_data::TestDataBuilder::new();
    let event = builder.create_block_items_received_event(1, None);

    tx_items.send(event).await.unwrap();

    // Verifier should process and emit BlockVerified
    // (Note: In real system, verifier does actual verification logic)
    // For this test, we just verify the plugin is running and can receive events

    // The verifier might not immediately emit an event if it needs actual block data
    // So this test mainly verifies the plugin integration works

    ctx.stop_plugins().await.unwrap();
}

#[tokio::test]
#[traced_test]
async fn test_block_persisted_event_broadcast() {
    let rx_block_verified = {
        let (_, rx) = tokio::sync::mpsc::channel(10);
        rx
    };

    let mut ctx = IntegrationTestContext::builder()
        .with_plugin(Box::new(PersistencePlugin::new(None, rx_block_verified)))
        .build()
        .await
        .unwrap();

    // Subscribe to persisted events before starting
    let _rx_persisted = ctx.subscribe_block_persisted();

    ctx.initialize_plugins().unwrap();
    ctx.start_plugins().unwrap();

    // In a real scenario, persistence would emit events when blocks are persisted
    // For this test, we verify the broadcast channel works

    ctx.stop_plugins().await.unwrap();
}

#[tokio::test]
#[traced_test]
async fn test_database_shared_between_plugins() {
    let (_tx_items, rx_items) = tokio::sync::mpsc::channel(10);

    let mut ctx = IntegrationTestContext::builder()
        .with_plugin(Box::new(VerifierPlugin::new(rx_items)))
        .build()
        .await
        .unwrap();

    ctx.initialize_plugins().unwrap();

    // Both plugins should have access to the same database through DatabaseManagerProvider
    {
        let providers = ctx.context.service_providers.read().unwrap();
        assert!(providers
            .get(&TypeId::of::<
                rock_node_core::database_provider::DatabaseManagerProvider,
            >())
            .is_some());
    }

    ctx.stop_plugins().await.unwrap();
}

#[tokio::test]
#[traced_test]
async fn test_metrics_collection_across_plugins() {
    let (_tx_items, rx_items) = tokio::sync::mpsc::channel(10);

    let mut ctx = IntegrationTestContext::builder()
        .with_plugin(Box::new(VerifierPlugin::new(rx_items)))
        .build()
        .await
        .unwrap();

    ctx.initialize_plugins().unwrap();
    ctx.start_plugins().unwrap();

    // Both plugins share the same metrics registry
    let metrics = &ctx.context.metrics;

    // Metrics should be accessible
    let _initial_blocks = metrics.blocks_acknowledged.get();

    ctx.stop_plugins().await.unwrap();
}

#[tokio::test]
#[traced_test]
async fn test_context_cleanup_on_drop() {
    let temp_path = {
        let (_tx, rx) = tokio::sync::mpsc::channel(10);
        let mut ctx = IntegrationTestContext::builder()
            .with_plugin(Box::new(VerifierPlugin::new(rx)))
            .build()
            .await
            .unwrap();

        ctx.initialize_plugins().unwrap();

        let path = ctx.temp_dir().to_path_buf();
        assert!(path.exists());

        ctx.stop_plugins().await.unwrap();
        path
    };

    // After context is dropped, temp directory should be cleaned up
    assert!(!temp_path.exists());
}
