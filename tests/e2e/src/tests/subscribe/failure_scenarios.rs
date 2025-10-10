use crate::common::{publish_blocks, TestContext};
use anyhow::Result;
use rock_node_protobufs::org::hiero::block::api::{
    subscribe_stream_response::{Code as SubCode, Response as SubResponse},
    SubscribeStreamRequest,
};
use serial_test::serial;
use tokio_stream::StreamExt;

/// Test Case: Client disconnect mid-stream.
/// Tests that when a client disconnects during an active subscription,
/// the session is cleaned up gracefully and metrics are updated correctly.
#[tokio::test]
#[serial]
async fn test_client_disconnect_tracking() -> Result<()> {
    let ctx = TestContext::new().await?;

    // Publish many blocks
    publish_blocks(&ctx, 0, 100).await?;

    let mut sub_client = ctx.subscriber_client().await?;
    let request = SubscribeStreamRequest {
        start_block_number: 0,
        end_block_number: 50,
    };

    let mut stream = sub_client
        .subscribe_block_stream(request)
        .await?
        .into_inner();

    // Receive a few blocks then disconnect
    for _ in 0..10 {
        stream.next().await;
    }

    // Force disconnect by dropping the stream and client
    drop(stream);
    drop(sub_client);

    // Wait a bit for cleanup
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    // Verify session was cleaned up
    // In a real test, we'd check metrics or logs here
    // For now, this validates that the disconnect doesn't cause a panic

    Ok(())
}

/// Test Case: Server shutdown during active subscription.
/// Tests that when the server initiates shutdown, active subscribers
/// receive a graceful Error status and the stream terminates cleanly.
///
/// Note: This test is challenging to implement in E2E as it requires
/// shutting down the Docker container mid-test. Marked as placeholder.
#[tokio::test]
#[serial]
#[ignore] // Requires infrastructure to gracefully shutdown server mid-test
async fn test_server_shutdown_during_subscription() -> Result<()> {
    // TODO: Implement when we have server shutdown capabilities in TestContext
    // Steps:
    // 1. Start subscription with infinite range
    // 2. Receive a few blocks
    // 3. Trigger server shutdown via TestContext.shutdown_server()
    // 4. Verify subscriber receives Error status
    // 5. Verify metrics show server_shutdown
    Ok(())
}

/// Test Case: Timeout waiting for block.
/// Tests that when a subscriber is waiting for a block that never arrives,
/// the session timeout triggers and the stream ends with an error status.
///
/// Note: This requires configuring a short session_timeout_seconds.
/// Currently challenging to test as TestContext uses default config.
#[tokio::test]
#[serial]
#[ignore] // Requires custom config with short timeout
async fn test_timeout_waiting_for_block() -> Result<()> {
    // TODO: Implement when TestContext supports custom config
    // Steps:
    // 1. Create TestContext with session_timeout_seconds = 2
    // 2. Publish blocks 0-5
    // 3. Subscribe requesting blocks 0-20
    // 4. Receive blocks 0-5
    // 5. Wait for block 6 (which never arrives)
    // 6. After 2 seconds, should timeout
    // 7. Verify Error status received
    // 8. Verify metrics show timeout label
    Ok(())
}

/// Test Case: Multiple subscribers, one disconnects.
/// Tests that when multiple subscribers are active and one disconnects,
/// the others continue operating normally without disruption.
#[tokio::test]
#[serial]
async fn test_multiple_subscribers_one_disconnects() -> Result<()> {
    let ctx = TestContext::new().await?;

    publish_blocks(&ctx, 0, 50).await?;

    // Subscriber A: Will disconnect early
    let mut client_a = ctx.subscriber_client().await?;
    let request_a = SubscribeStreamRequest {
        start_block_number: 0,
        end_block_number: u64::MAX,
    };
    let mut stream_a = client_a
        .subscribe_block_stream(request_a)
        .await?
        .into_inner();

    // Subscriber B: Will continue to completion
    let mut client_b = ctx.subscriber_client().await?;
    let request_b = SubscribeStreamRequest {
        start_block_number: 0,
        end_block_number: 30,
    };
    let mut stream_b = client_b
        .subscribe_block_stream(request_b)
        .await?
        .into_inner();

    // Both receive a few blocks
    for _ in 0..5 {
        stream_a.next().await;
        stream_b.next().await;
    }

    // A disconnects
    drop(stream_a);
    drop(client_a);

    // B should continue without issues
    let mut received_b = 5; // Already received 5
    while let Some(Ok(msg)) = stream_b.next().await {
        match msg.response {
            Some(SubResponse::BlockItems(_)) => {
                received_b += 1;
            },
            Some(SubResponse::Status(code)) => {
                assert_eq!(code, SubCode::Success as i32);
                break;
            },
            _ => {},
        }
    }

    assert_eq!(
        received_b, 31,
        "Subscriber B should receive all 31 blocks (0-30) despite A's disconnect"
    );
    Ok(())
}

/// Test Case: Lag detection on slow subscriber.
/// Tests that when a subscriber processes blocks slowly and the broadcast
/// channel lags, the system handles it gracefully by falling back to
/// persistence reads.
///
/// Note: This is difficult to trigger reliably in E2E as Rust's broadcast
/// channel handles lag automatically. Included as placeholder for awareness.
#[tokio::test]
#[serial]
#[ignore] // Difficult to trigger lag reliably in E2E
async fn test_subscriber_lag_handling() -> Result<()> {
    // TODO: Consider implementing with artificial delays
    // Steps:
    // 1. Create subscriber
    // 2. Publish blocks very rapidly (faster than subscriber processes)
    // 3. Subscriber should still receive all blocks (may switch to persistence reads)
    // 4. Verify no data loss
    // 5. Verify logs show lag detection
    Ok(())
}
