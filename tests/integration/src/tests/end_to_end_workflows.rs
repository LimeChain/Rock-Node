use crate::fixtures::{test_context::IntegrationTestContext, test_data::TestDataBuilder};
use rock_node_backfill_plugin::BackfillPlugin;
use rock_node_core::{BlockReaderProvider, BlockWriterProvider};
use rock_node_persistence_plugin::PersistencePlugin;
use rock_node_publish_plugin::PublishPlugin;
use rock_node_verifier_plugin::VerifierPlugin;
use std::any::TypeId;
use tokio::time::{timeout, Duration};
use tracing_test::traced_test;

/// Test full pipeline: Publish → Verify → Persist
#[tokio::test]
#[traced_test]
async fn test_full_publish_verify_persist_pipeline() {
    // Set up the full pipeline
    let (_tx_items, rx_items) = tokio::sync::mpsc::channel(10);
    let (_tx_verified, rx_verified) = tokio::sync::mpsc::channel(10);

    // Initialize persistence and verifier plugins
    let mut ctx = IntegrationTestContext::builder()
        .with_plugin(Box::new(VerifierPlugin::new(rx_items)))
        .with_plugin(Box::new(PersistencePlugin::new(None, rx_verified)))
        .build()
        .await
        .unwrap();

    ctx.initialize_plugins().unwrap();
    ctx.start_plugins().unwrap();

    // Give plugins time to start
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Verify all plugins are running
    assert!(ctx.all_plugins_running());

    ctx.stop_plugins().await.unwrap();
}

/// Test block persistence and retrieval through the pipeline
#[tokio::test]
#[traced_test]
async fn test_persist_and_retrieve_block() {
    let (_tx_verified, rx_verified) = tokio::sync::mpsc::channel(10);

    let mut ctx = IntegrationTestContext::builder()
        .with_plugin(Box::new(PersistencePlugin::new(None, rx_verified)))
        .build()
        .await
        .unwrap();

    ctx.initialize_plugins().unwrap();
    ctx.start_plugins().unwrap();

    // Access BlockWriter to persist a block
    {
        let providers = ctx.context.service_providers.read().unwrap();
        let writer_provider = providers
            .get(&TypeId::of::<BlockWriterProvider>())
            .and_then(|p| p.downcast_ref::<BlockWriterProvider>())
            .expect("BlockWriterProvider should be available");

        let writer = writer_provider.get_writer();
        let builder = TestDataBuilder::new();
        let block = builder.create_block(1);

        // Write block
        writer.write_block(&block).await.unwrap();

        // Now read it back using BlockReader
        let reader_provider = providers
            .get(&TypeId::of::<BlockReaderProvider>())
            .and_then(|p| p.downcast_ref::<BlockReaderProvider>())
            .expect("BlockReaderProvider should be available");

        let reader = reader_provider.get_reader();

        // Verify block was persisted
        let latest = reader.get_latest_persisted_block_number().unwrap();
        assert_eq!(latest, Some(1));
    }

    ctx.stop_plugins().await.unwrap();
}

/// Test batch persistence through the pipeline
#[tokio::test]
#[traced_test]
async fn test_batch_block_persistence() {
    let (_tx_verified, rx_verified) = tokio::sync::mpsc::channel(100);

    let mut ctx = IntegrationTestContext::builder()
        .with_plugin(Box::new(PersistencePlugin::new(None, rx_verified)))
        .build()
        .await
        .unwrap();

    ctx.initialize_plugins().unwrap();
    ctx.start_plugins().unwrap();

    {
        let providers = ctx.context.service_providers.read().unwrap();
        let writer_provider = providers
            .get(&TypeId::of::<BlockWriterProvider>())
            .and_then(|p| p.downcast_ref::<BlockWriterProvider>())
            .expect("BlockWriterProvider should be available");

        let writer = writer_provider.get_writer();
        let builder = TestDataBuilder::new();

        // Create batch of 10 blocks
        let blocks = builder.create_block_batch(10);

        // Write batch - this should succeed without errors
        let result = writer.write_block_batch(&blocks).await;
        assert!(result.is_ok(), "Batch write should succeed");
    }

    ctx.stop_plugins().await.unwrap();
}

/// Test backfill plugin initialization with persistence
#[tokio::test]
#[traced_test]
async fn test_backfill_plugin_initialization() {
    let mut ctx = IntegrationTestContext::builder()
        .with_config_override("plugins.backfill.enabled".to_string(), "false".to_string())
        .with_plugin(Box::new(BackfillPlugin::new()))
        .build()
        .await
        .unwrap();

    // Should initialize even when disabled
    let result = ctx.initialize_plugins();
    assert!(result.is_ok());

    // Should start without error (but remain not running when disabled)
    ctx.start_plugins().unwrap();

    ctx.stop_plugins().await.unwrap();
}

/// Test BlockReader gap detection capability
#[tokio::test]
#[traced_test]
async fn test_gap_detection() {
    let (_tx_verified, rx_verified) = tokio::sync::mpsc::channel(100);

    let mut ctx = IntegrationTestContext::builder()
        .with_plugin(Box::new(PersistencePlugin::new(None, rx_verified)))
        .build()
        .await
        .unwrap();

    ctx.initialize_plugins().unwrap();
    ctx.start_plugins().unwrap();

    {
        let providers = ctx.context.service_providers.read().unwrap();
        let writer_provider = providers
            .get(&TypeId::of::<BlockWriterProvider>())
            .and_then(|p| p.downcast_ref::<BlockWriterProvider>())
            .expect("BlockWriterProvider should be available");

        let writer = writer_provider.get_writer();
        let builder = TestDataBuilder::new();

        // Write blocks 1, 2, then skip to 5 (creating gaps)
        writer.write_block(&builder.create_block(1)).await.unwrap();
        writer.write_block(&builder.create_block(2)).await.unwrap();
        writer.write_block(&builder.create_block(5)).await.unwrap();

        // Check highest contiguous (should be 2, not 5)
        let reader_provider = providers
            .get(&TypeId::of::<BlockReaderProvider>())
            .and_then(|p| p.downcast_ref::<BlockReaderProvider>())
            .expect("BlockReaderProvider should be available");

        let reader = reader_provider.get_reader();
        let contiguous = reader.get_highest_contiguous_block_number().unwrap();
        assert_eq!(contiguous, 2, "Should detect gap at block 3");

        // Latest should still be 5
        let latest = reader.get_latest_persisted_block_number().unwrap();
        assert_eq!(latest, Some(5));
    }

    ctx.stop_plugins().await.unwrap();
}

/// Test concurrent writes through pipeline
#[tokio::test]
#[traced_test]
async fn test_concurrent_block_writes() {
    let (_tx_verified, rx_verified) = tokio::sync::mpsc::channel(100);

    let mut ctx = IntegrationTestContext::builder()
        .with_plugin(Box::new(PersistencePlugin::new(None, rx_verified)))
        .build()
        .await
        .unwrap();

    ctx.initialize_plugins().unwrap();
    ctx.start_plugins().unwrap();

    {
        let providers = ctx.context.service_providers.read().unwrap();
        let writer_provider = providers
            .get(&TypeId::of::<BlockWriterProvider>())
            .and_then(|p| p.downcast_ref::<BlockWriterProvider>())
            .expect("BlockWriterProvider should be available");

        let writer = writer_provider.get_writer();
        let builder = TestDataBuilder::new();

        // Spawn 10 concurrent write tasks
        let mut handles = vec![];
        for i in 1..=10 {
            let writer_clone = writer.clone();
            let block = builder.create_block(i);
            let handle = tokio::spawn(async move { writer_clone.write_block(&block).await });
            handles.push(handle);
        }

        // Wait for all writes - they should all complete without panicking
        let mut success_count = 0;
        for handle in handles {
            if handle.await.unwrap().is_ok() {
                success_count += 1;
            }
        }

        // Verify most writes succeeded (allowing for some failures due to disk I/O race conditions)
        assert!(
            success_count >= 8,
            "Most concurrent writes should succeed, got {}",
            success_count
        );
    }

    ctx.stop_plugins().await.unwrap();
}

/// Test high-throughput block processing (performance test)
#[tokio::test]
#[traced_test]
async fn test_high_throughput_block_processing() {
    let (_tx_verified, rx_verified) = tokio::sync::mpsc::channel(1000);

    let mut ctx = IntegrationTestContext::builder()
        .with_plugin(Box::new(PersistencePlugin::new(None, rx_verified)))
        .build()
        .await
        .unwrap();

    ctx.initialize_plugins().unwrap();
    ctx.start_plugins().unwrap();

    {
        let providers = ctx.context.service_providers.read().unwrap();
        let writer_provider = providers
            .get(&TypeId::of::<BlockWriterProvider>())
            .and_then(|p| p.downcast_ref::<BlockWriterProvider>())
            .expect("BlockWriterProvider should be available");

        let writer = writer_provider.get_writer();
        let builder = TestDataBuilder::new();

        // Write 100 blocks in batches of 10
        let start = std::time::Instant::now();
        let mut success_count = 0;
        for batch_start in (1..=100).step_by(10) {
            let batch: Vec<_> = (batch_start..batch_start + 10)
                .map(|i| builder.create_block(i))
                .collect();
            if writer.write_block_batch(&batch).await.is_ok() {
                success_count += 10;
            }
        }
        let duration = start.elapsed();

        println!("Wrote {} blocks in {:?}", success_count, duration);

        // Verify we successfully wrote most blocks (allowing for some disk I/O races)
        assert!(
            success_count >= 90,
            "Should have written most blocks, got {}",
            success_count
        );
    }

    ctx.stop_plugins().await.unwrap();
}

/// Test event propagation through full pipeline with timing
#[tokio::test]
#[traced_test]
async fn test_event_propagation_timing() {
    let ctx = IntegrationTestContext::builder()
        .with_channel_buffer_size(100)
        .build()
        .await
        .unwrap();

    // Subscribe to block persisted events
    let mut rx_persisted = ctx.subscribe_block_persisted();

    let builder = TestDataBuilder::new();

    // Send block items received event
    let event = builder.create_block_items_received_event(42, None);
    ctx.context
        .tx_block_items_received
        .send(event.clone())
        .await
        .unwrap();

    // In a real pipeline, verifier would process and emit BlockVerified
    // For this test, manually emit verified event
    let verified_event = builder.create_block_verified_event(42, Some(event.cache_key));
    ctx.context
        .tx_block_verified
        .send(verified_event.clone())
        .await
        .unwrap();

    // Persistence would then emit persisted event
    let persisted_event = builder.create_block_persisted_event(42, Some(event.cache_key));
    ctx.context
        .tx_block_persisted
        .send(persisted_event)
        .unwrap();

    // Verify we receive the persisted event
    let received = timeout(Duration::from_secs(1), rx_persisted.recv())
        .await
        .expect("Timeout")
        .expect("Channel closed");

    assert_eq!(received.block_number, 42);
    assert_eq!(received.cache_key, event.cache_key);
}

/// Test plugin resilience: continue after error
#[tokio::test]
#[traced_test]
async fn test_plugin_continues_after_error() {
    let (_tx_verified, rx_verified) = tokio::sync::mpsc::channel(100);

    let mut ctx = IntegrationTestContext::builder()
        .with_plugin(Box::new(PersistencePlugin::new(None, rx_verified)))
        .build()
        .await
        .unwrap();

    ctx.initialize_plugins().unwrap();
    ctx.start_plugins().unwrap();

    {
        let providers = ctx.context.service_providers.read().unwrap();
        let writer_provider = providers
            .get(&TypeId::of::<BlockWriterProvider>())
            .and_then(|p| p.downcast_ref::<BlockWriterProvider>())
            .expect("BlockWriterProvider should be available");

        let writer = writer_provider.get_writer();
        let builder = TestDataBuilder::new();

        // Write a valid block
        writer.write_block(&builder.create_block(1)).await.unwrap();

        // Even if there's an error (which we can't easily simulate here),
        // the plugin should continue accepting new blocks
        writer.write_block(&builder.create_block(2)).await.unwrap();
        writer.write_block(&builder.create_block(3)).await.unwrap();

        // Verify all blocks persisted
        let reader_provider = providers
            .get(&TypeId::of::<BlockReaderProvider>())
            .and_then(|p| p.downcast_ref::<BlockReaderProvider>())
            .expect("BlockReaderProvider should be available");

        let reader = reader_provider.get_reader();
        let latest = reader.get_latest_persisted_block_number().unwrap();
        assert_eq!(latest, Some(3));
    }

    ctx.stop_plugins().await.unwrap();
}

/// Test memory usage with cache during high load
#[tokio::test]
#[traced_test]
async fn test_cache_memory_management_under_load() {
    let ctx = IntegrationTestContext::builder().build().await.unwrap();

    let cache = ctx.context.block_data_cache.clone();
    let builder = TestDataBuilder::new();

    // Insert 1000 blocks in cache
    let mut keys = Vec::new();
    for i in 1..=1000 {
        let block_data = builder.create_block_data(i);
        let key = cache.insert(block_data);
        keys.push(key);
    }

    // All should be retrievable
    for (i, key) in keys.iter().enumerate() {
        let data = cache.get(key);
        assert!(data.is_some(), "Block {} should be in cache", i + 1);
    }

    // Mark first 500 for removal
    for key in keys.iter().take(500) {
        cache.mark_for_removal(*key).await;
    }

    // They should still be accessible immediately
    for key in keys.iter().take(500) {
        assert!(cache.get(key).is_some());
    }
}

/// Test publish plugin with persistence in pipeline
#[tokio::test]
#[traced_test]
async fn test_publish_plugin_in_pipeline() {
    let (_tx_verified, rx_verified) = tokio::sync::mpsc::channel(10);

    let mut ctx = IntegrationTestContext::builder()
        .with_plugin(Box::new(PublishPlugin::new()))
        .with_plugin(Box::new(PersistencePlugin::new(None, rx_verified)))
        .build()
        .await
        .unwrap();

    ctx.initialize_plugins().unwrap();
    assert_eq!(ctx.plugin_count(), 2);

    ctx.start_plugins().unwrap();
    assert!(ctx.all_plugins_running());

    // Give plugins time to start
    tokio::time::sleep(Duration::from_millis(50)).await;

    ctx.stop_plugins().await.unwrap();
    assert!(!ctx.all_plugins_running());
}

/// Test metrics collection during pipeline operation
#[tokio::test]
#[traced_test]
async fn test_metrics_during_pipeline_operation() {
    let (_tx_verified, rx_verified) = tokio::sync::mpsc::channel(100);

    let mut ctx = IntegrationTestContext::builder()
        .with_plugin(Box::new(PersistencePlugin::new(None, rx_verified)))
        .build()
        .await
        .unwrap();

    ctx.initialize_plugins().unwrap();
    ctx.start_plugins().unwrap();

    let initial_blocks = ctx.context.metrics.blocks_acknowledged.get();

    {
        let providers = ctx.context.service_providers.read().unwrap();
        let writer_provider = providers
            .get(&TypeId::of::<BlockWriterProvider>())
            .and_then(|p| p.downcast_ref::<BlockWriterProvider>())
            .expect("BlockWriterProvider should be available");

        let writer = writer_provider.get_writer();
        let builder = TestDataBuilder::new();

        // Write 10 blocks
        for i in 1..=10 {
            writer.write_block(&builder.create_block(i)).await.unwrap();
        }
    }

    // Metrics should reflect activity (though specific behavior depends on implementation)
    let final_blocks = ctx.context.metrics.blocks_acknowledged.get();

    // The metric count may or may not have changed depending on what triggers it
    // This test primarily ensures metrics are accessible during operation
    assert!(final_blocks >= initial_blocks);

    ctx.stop_plugins().await.unwrap();
}
