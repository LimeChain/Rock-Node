use crate::common::{publish_blocks, TestContext};
use anyhow::Result;
use rock_node_protobufs::org::hiero::block::api::{
    subscribe_stream_response::{Code as SubCode, Response as SubResponse},
    SubscribeStreamRequest,
};
use serial_test::serial;
use std::sync::Arc;
use tokio_stream::StreamExt;

/// Helper to extract the block number from a `BlockItems` response.
fn header_number(item_set: &rock_node_protobufs::org::hiero::block::api::BlockItemSet) -> u64 {
    use rock_node_protobufs::com::hedera::hapi::block::stream::block_item::Item;
    item_set
        .block_items
        .iter()
        .find_map(|item| match &item.item {
            Some(Item::BlockHeader(h)) => Some(h.number),
            _ => None,
        })
        .expect("Block items must contain a header")
}

/// Test Case: Validation error when start > end.
/// Tests that the new validation logic correctly rejects requests where
/// the start block is after the end block.
#[tokio::test]
#[serial]
async fn test_invalid_end_block_number() -> Result<()> {
    let ctx = TestContext::new().await?;

    let mut sub_client = ctx.subscriber_client().await?;
    let request = SubscribeStreamRequest {
        start_block_number: 10,
        end_block_number: 5,
    };

    let mut stream = sub_client
        .subscribe_block_stream(request)
        .await?
        .into_inner();

    let first = stream.next().await.expect("Expected a response")?;
    match first.response {
        Some(SubResponse::Status(code)) => {
            assert_eq!(
                code,
                SubCode::InvalidEndBlockNumber as i32,
                "Expected InvalidEndBlockNumber error"
            );
        },
        other => panic!("Unexpected response: {:?}", other),
    }
    assert!(
        stream.next().await.is_none(),
        "Stream should have terminated"
    );
    Ok(())
}

/// Test Case: Start block too far in the future.
/// Tests that the validation logic rejects requests where the start block
/// exceeds (latest + max_future_block_lookahead).
#[tokio::test]
#[serial]
async fn test_validation_start_too_far_in_future() -> Result<()> {
    // Custom config with explicit lookahead to make test deterministic
    let custom_config = r#"
[core]
log_level = "info"
database_path = "/app/data"
start_block_number = 0
grpc_address = "0.0.0.0"
grpc_port = 50051

[plugins]
    [plugins.observability]
    enabled = true
    listen_address = "0.0.0.0:8080"

    [plugins.persistence_service]
    enabled = true
    cold_storage_path = "/app/data/cold"
    hot_storage_block_count = 10000
    archive_batch_size = 1000

    [plugins.verification_service]
    enabled = true

    [plugins.state_management_service]
    enabled = true

    [plugins.publish_service]
    enabled = true
    max_concurrent_streams = 10

    [plugins.subscriber_service]
    enabled = true
    max_concurrent_streams = 10
    max_future_block_lookahead = 250
    session_timeout_seconds = 60

    [plugins.block_access_service]
    enabled = true

    [plugins.server_status_service]
    enabled = true

    [plugins.query_service]
    enabled = true
"#;

    let ctx = TestContext::with_config(Some(custom_config), None).await?;

    // Publish blocks 0-10 (latest = 10)
    publish_blocks(&ctx, 0, 10).await?;

    let mut sub_client = ctx.subscriber_client().await?;

    // Try to subscribe starting at block 500
    // max_future_block_lookahead = 250 (from custom config)
    // Max permitted start = 10 + 250 = 260
    // 500 > 260, so should be rejected
    let request = SubscribeStreamRequest {
        start_block_number: 500,
        end_block_number: u64::MAX,
    };

    let mut stream = sub_client
        .subscribe_block_stream(request)
        .await?
        .into_inner();

    // Should get NotAvailable error status
    let response = tokio::time::timeout(tokio::time::Duration::from_secs(5), stream.next())
        .await?
        .expect("Expected response")?;

    match response.response {
        Some(SubResponse::Status(code)) => {
            assert_eq!(
                code,
                SubCode::NotAvailable as i32,
                "Expected NotAvailable for start beyond lookahead"
            );
        },
        Some(SubResponse::BlockItems(_)) => {
            panic!("Should not receive blocks when start (500) is beyond lookahead (260)");
        },
        other => panic!("Expected NotAvailable status, got {:?}", other),
    }

    // Stream should terminate
    assert!(stream.next().await.is_none());
    Ok(())
}

/// Test Case: Pure live streaming (start=MAX, end=MAX) from empty node.
/// Tests that pure live mode is accepted even when no blocks exist yet,
/// and that the subscriber receives blocks as they arrive.
#[tokio::test]
#[serial]
async fn test_pure_live_streaming() -> Result<()> {
    let ctx = TestContext::new().await?;

    // Start with NO blocks published
    let mut sub_client = ctx.subscriber_client().await?;
    let request = SubscribeStreamRequest {
        start_block_number: u64::MAX,
        end_block_number: u64::MAX,
    };

    let mut stream = sub_client
        .subscribe_block_stream(request)
        .await?
        .into_inner();

    // Spawn task to publish blocks after subscription starts
    let ctx_arc = Arc::new(ctx);
    let ctx_clone = Arc::clone(&ctx_arc);
    tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        publish_blocks(&ctx_clone, 0, 4).await.unwrap();
    });

    // Should receive blocks as they arrive
    let mut received = Vec::new();
    for _ in 0..5 {
        if let Some(Ok(msg)) =
            tokio::time::timeout(tokio::time::Duration::from_secs(5), stream.next()).await?
        {
            if let Some(SubResponse::BlockItems(set)) = msg.response {
                received.push(header_number(&set));
            }
        }
    }

    assert_eq!(
        received,
        vec![0, 1, 2, 3, 4],
        "Should receive blocks 0-4 in order"
    );
    Ok(())
}

/// Test Case: From earliest to specific end (start=MAX, end=N).
/// Tests that start=MAX resolves to the earliest available block,
/// and that finite range works correctly.
#[tokio::test]
#[serial]
async fn test_from_earliest_to_end() -> Result<()> {
    let ctx = TestContext::new().await?;

    // Publish blocks 10-20 (earliest = 10)
    publish_blocks(&ctx, 10, 20).await?;

    let mut sub_client = ctx.subscriber_client().await?;
    let request = SubscribeStreamRequest {
        start_block_number: u64::MAX, // Should resolve to earliest (10)
        end_block_number: 15,
    };

    let mut stream = sub_client
        .subscribe_block_stream(request)
        .await?
        .into_inner();

    let mut received = Vec::new();
    while let Some(Ok(msg)) = stream.next().await {
        match msg.response {
            Some(SubResponse::BlockItems(set)) => {
                received.push(header_number(&set));
            },
            Some(SubResponse::Status(code)) => {
                assert_eq!(code, SubCode::Success as i32);
                break;
            },
            _ => {},
        }
    }

    // Should receive blocks 10-15 (earliest to requested end)
    assert_eq!(
        received,
        (10..=15).collect::<Vec<u64>>(),
        "Should receive blocks from earliest (10) to end (15)"
    );
    Ok(())
}

/// Test Case: Request start before earliest available block.
/// Tests that the validation logic correctly rejects requests where
/// the start block is before the earliest available block.
#[tokio::test]
#[serial]
async fn test_validation_start_before_earliest() -> Result<()> {
    let ctx = TestContext::new().await?;

    // Publish blocks 50-60 (earliest = 50)
    publish_blocks(&ctx, 50, 60).await?;

    let mut sub_client = ctx.subscriber_client().await?;

    // Try to subscribe from block 10 (before earliest)
    let request = SubscribeStreamRequest {
        start_block_number: 10,
        end_block_number: u64::MAX,
    };

    let mut stream = sub_client
        .subscribe_block_stream(request)
        .await?
        .into_inner();

    let response = stream.next().await.expect("Expected response")?;
    match response.response {
        Some(SubResponse::Status(code)) => {
            assert_eq!(
                code,
                SubCode::InvalidStartBlockNumber as i32,
                "Expected InvalidStartBlockNumber for start before earliest"
            );
        },
        other => panic!("Expected InvalidStartBlockNumber, got {:?}", other),
    }

    // Stream should terminate
    assert!(stream.next().await.is_none());
    Ok(())
}

/// Test Case: Future start with no blocks available.
/// Tests that a future start block is accepted when no blocks exist yet
/// (within lookahead), and the subscriber waits for blocks to arrive.
#[tokio::test]
#[serial]
async fn test_future_start_no_blocks_available() -> Result<()> {
    let ctx = TestContext::new().await?;

    // NO blocks published yet
    let mut sub_client = ctx.subscriber_client().await?;
    let request = SubscribeStreamRequest {
        start_block_number: 100, // Future block (within lookahead from 0)
        end_block_number: u64::MAX,
    };

    let mut stream = sub_client
        .subscribe_block_stream(request)
        .await?
        .into_inner();

    // Spawn task to publish blocks starting at 100
    let ctx_arc = Arc::new(ctx);
    let ctx_clone = Arc::clone(&ctx_arc);
    tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        publish_blocks(&ctx_clone, 100, 104).await.unwrap();
    });

    // Should receive blocks starting at 100
    let mut received = Vec::new();
    for _ in 0..5 {
        if let Some(Ok(msg)) =
            tokio::time::timeout(tokio::time::Duration::from_secs(5), stream.next()).await?
        {
            if let Some(SubResponse::BlockItems(set)) = msg.response {
                received.push(header_number(&set));
            }
        }
    }

    assert_eq!(
        received,
        vec![100, 101, 102, 103, 104],
        "Should receive blocks starting at future block 100"
    );
    Ok(())
}

/// Test Case: From earliest but end is before earliest.
/// Tests that when requesting start=MAX with an end block before the earliest
/// available, the request is either rejected or completes with no blocks.
#[tokio::test]
#[serial]
async fn test_validation_from_earliest_but_end_before_earliest() -> Result<()> {
    let ctx = TestContext::new().await?;

    // Publish blocks 10-20 (earliest = 10)
    publish_blocks(&ctx, 10, 20).await?;

    let mut sub_client = ctx.subscriber_client().await?;

    // Request from earliest (10) to 5 (before earliest)
    let request = SubscribeStreamRequest {
        start_block_number: u64::MAX, // Will resolve to 10
        end_block_number: 5,          // Before earliest
    };

    let mut stream = sub_client
        .subscribe_block_stream(request)
        .await?
        .into_inner();

    // Should receive an error status (could be NotAvailable or other error)
    // The important thing is that it doesn't successfully stream blocks
    let response = stream.next().await.expect("Expected response")?;
    match response.response {
        Some(SubResponse::Status(code)) => {
            // Accept any non-success error code
            assert_ne!(
                code,
                SubCode::Success as i32,
                "Should not succeed when end < earliest"
            );
        },
        Some(SubResponse::BlockItems(_)) => {
            panic!("Should not receive blocks when end (5) < earliest (10)");
        },
        other => panic!("Unexpected response: {:?}", other),
    }

    // Stream should terminate
    assert!(stream.next().await.is_none());
    Ok(())
}
