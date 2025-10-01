use crate::fixtures::{test_context::IntegrationTestContext, test_data::TestDataBuilder};
use tokio::time::{timeout, Duration};
use tracing_test::traced_test;

#[tokio::test]
#[traced_test]
async fn test_block_items_received_propagation() {
    let mut ctx = IntegrationTestContext::builder().build().await.unwrap();

    // Take ownership of the receiver
    let mut rx = ctx.take_block_items_received_rx().unwrap();

    // Send an event
    let builder = TestDataBuilder::new();
    let event = builder.create_block_items_received_event(1, None);

    ctx.context
        .tx_block_items_received
        .send(event)
        .await
        .unwrap();

    // Event should be received
    let received = timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("Timeout waiting for event")
        .expect("Channel closed");

    assert_eq!(received.block_number, 1);
    assert_eq!(received.cache_key, event.cache_key);
}

#[tokio::test]
#[traced_test]
async fn test_block_verified_propagation() {
    let mut ctx = IntegrationTestContext::builder().build().await.unwrap();

    let mut rx = ctx.take_block_verified_rx().unwrap();

    let builder = TestDataBuilder::new();
    let event = builder.create_block_verified_event(2, None);

    ctx.context.tx_block_verified.send(event).await.unwrap();

    let received = timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("Timeout")
        .expect("Channel closed");

    assert_eq!(received.block_number, 2);
}

#[tokio::test]
#[traced_test]
async fn test_block_persisted_broadcast() {
    let ctx = IntegrationTestContext::builder().build().await.unwrap();

    // Create multiple subscribers
    let mut rx1 = ctx.subscribe_block_persisted();
    let mut rx2 = ctx.subscribe_block_persisted();
    let mut rx3 = ctx.subscribe_block_persisted();

    let builder = TestDataBuilder::new();
    let event = builder.create_block_persisted_event(3, None);

    // Send event once
    ctx.context.tx_block_persisted.send(event).unwrap();

    // All subscribers should receive it
    let received1 = timeout(Duration::from_secs(1), rx1.recv())
        .await
        .expect("Timeout")
        .expect("Channel closed");
    let received2 = timeout(Duration::from_secs(1), rx2.recv())
        .await
        .expect("Timeout")
        .expect("Channel closed");
    let received3 = timeout(Duration::from_secs(1), rx3.recv())
        .await
        .expect("Timeout")
        .expect("Channel closed");

    assert_eq!(received1.block_number, 3);
    assert_eq!(received2.block_number, 3);
    assert_eq!(received3.block_number, 3);
    assert_eq!(received1.cache_key, received2.cache_key);
    assert_eq!(received2.cache_key, received3.cache_key);
}

#[tokio::test]
#[traced_test]
async fn test_event_ordering() {
    let mut ctx = IntegrationTestContext::builder().build().await.unwrap();

    let mut rx = ctx.take_block_items_received_rx().unwrap();

    let builder = TestDataBuilder::new();

    // Send multiple events in order
    for i in 1..=5 {
        let event = builder.create_block_items_received_event(i, None);
        ctx.context
            .tx_block_items_received
            .send(event)
            .await
            .unwrap();
    }

    // Receive in order
    for expected in 1..=5 {
        let received = timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("Timeout")
            .expect("Channel closed");
        assert_eq!(received.block_number, expected);
    }
}

#[tokio::test]
#[traced_test]
async fn test_cache_lifecycle() {
    let ctx = IntegrationTestContext::builder().build().await.unwrap();

    let cache = &ctx.context.block_data_cache;
    let builder = TestDataBuilder::new();

    // Insert block data
    let block_data = builder.create_block_data(100);
    let key = cache.insert(block_data.clone());

    // Should be retrievable
    let retrieved = cache.get(&key).unwrap();
    assert_eq!(retrieved.block_number, 100);

    // Mark for removal
    cache.mark_for_removal(key).await;

    // Should still be accessible immediately
    assert!(cache.get(&key).is_some());
}

#[tokio::test]
#[traced_test]
async fn test_multiple_events_same_cache_key() {
    let ctx = IntegrationTestContext::builder().build().await.unwrap();

    let cache = &ctx.context.block_data_cache;
    let builder = TestDataBuilder::new();

    // Insert block data
    let block_data = builder.create_block_data(10);
    let cache_key = cache.insert(block_data.clone());

    // Create events with the same cache key
    let event1 = builder.create_block_items_received_event(10, Some(cache_key));
    let event2 = builder.create_block_verified_event(10, Some(cache_key));
    let event3 = builder.create_block_persisted_event(10, Some(cache_key));

    // All should reference the same cache entry
    assert_eq!(event1.cache_key, cache_key);
    assert_eq!(event2.cache_key, cache_key);
    assert_eq!(event3.cache_key, cache_key);

    // Cache data should be retrievable
    let retrieved = cache.get(&cache_key).unwrap();
    assert_eq!(retrieved.block_number, 10);
}

#[tokio::test]
#[traced_test]
async fn test_channel_buffer_overflow_handling() {
    // Create context with small buffer
    let mut ctx = IntegrationTestContext::builder()
        .with_channel_buffer_size(2)
        .build()
        .await
        .unwrap();

    let mut rx = ctx.take_block_items_received_rx().unwrap();
    let builder = TestDataBuilder::new();

    // Send events up to buffer size
    for i in 1..=2 {
        let event = builder.create_block_items_received_event(i, None);
        ctx.context
            .tx_block_items_received
            .send(event)
            .await
            .unwrap();
    }

    // Buffer should be full, this should not block in test
    // (in real system, sender would await or handle backpressure)

    // Consume one event
    let _ = rx.recv().await.unwrap();

    // Now there's room for one more
    let event = builder.create_block_items_received_event(3, None);
    ctx.context
        .tx_block_items_received
        .send(event)
        .await
        .unwrap();
}

#[tokio::test]
#[traced_test]
async fn test_broadcast_late_subscriber() {
    let ctx = IntegrationTestContext::builder().build().await.unwrap();

    let builder = TestDataBuilder::new();

    // Send an event before subscribing (will be dropped, no subscribers yet)
    let event1 = builder.create_block_persisted_event(1, None);
    let _ = ctx.context.tx_block_persisted.send(event1); // May fail if no subscribers

    // Subscribe after event was sent
    let mut rx = ctx.subscribe_block_persisted();

    // Send another event
    let event2 = builder.create_block_persisted_event(2, None);
    ctx.context.tx_block_persisted.send(event2).unwrap();

    // Late subscriber should only see event2
    let received = timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("Timeout")
        .expect("Channel closed");
    assert_eq!(received.block_number, 2);
}

#[tokio::test]
#[traced_test]
async fn test_concurrent_event_sending() {
    let mut ctx = IntegrationTestContext::builder().build().await.unwrap();

    let mut rx = ctx.take_block_items_received_rx().unwrap();
    let tx = ctx.context.tx_block_items_received.clone();

    let builder = TestDataBuilder::new();

    // Spawn multiple tasks sending events
    let mut handles = vec![];
    for i in 1..=10 {
        let tx_clone = tx.clone();
        let event = builder.create_block_items_received_event(i, None);
        let handle = tokio::spawn(async move { tx_clone.send(event).await });
        handles.push(handle);
    }

    // All sends should succeed
    for handle in handles {
        handle.await.unwrap().unwrap();
    }

    // Receive all events (order may vary due to concurrency)
    let mut received_numbers = vec![];
    for _ in 1..=10 {
        let event = timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("Timeout")
            .expect("Channel closed");
        received_numbers.push(event.block_number);
    }

    // Sort and verify all were received
    received_numbers.sort_unstable();
    assert_eq!(received_numbers, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
}

#[tokio::test]
#[traced_test]
async fn test_event_with_cache_integration() {
    let mut ctx = IntegrationTestContext::builder().build().await.unwrap();

    let cache = ctx.context.block_data_cache.clone();
    let mut rx = ctx.take_block_verified_rx().unwrap();
    let builder = TestDataBuilder::new();

    // Insert data in cache
    let block_data = builder.create_block_data(50);
    let cache_key = cache.insert(block_data.clone());

    // Send event referencing that cache key
    let event = builder.create_block_verified_event(50, Some(cache_key));
    ctx.context.tx_block_verified.send(event).await.unwrap();

    // Receive event
    let received_event = rx.recv().await.unwrap();

    // Use cache key from event to retrieve data
    let retrieved_data = cache.get(&received_event.cache_key).unwrap();
    assert_eq!(retrieved_data.block_number, 50);
    assert_eq!(retrieved_data.contents, block_data.contents);
}
