use crate::common::{publish_blocks, TestContext};
use anyhow::Result;
use rock_node_protobufs::org::hiero::block::api::{
    block_request::BlockSpecifier, block_response, BlockRequest, ServerStatusRequest,
};
use serial_test::serial;
use std::time::Duration;

/// Test Case: Publish blocks with a gap and verify server status.
/// Objective: Ensure the node can ingest a "future" block, creating a gap,
/// and that the server status accurately reflects the new latest block without
/// losing track of the earliest block.
#[tokio::test]
#[serial]
async fn test_publish_with_gap_and_verify_status() -> Result<()> {
    let ctx = TestContext::new().await?;

    // 1. Publish initial contiguous blocks
    publish_blocks(&ctx, 0, 9).await?;

    // 2. Publish a block that creates a gap (blocks 10-14 are missing)
    publish_blocks(&ctx, 15, 15).await?;

    // 3. Verify server status
    let mut status_client = ctx.status_client().await?;
    let response = status_client
        .server_status(ServerStatusRequest::default())
        .await?
        .into_inner();

    assert_eq!(
        response.first_available_block, 0,
        "First available block should remain 0"
    );
    assert_eq!(
        response.last_available_block, 15,
        "Last available block should now be 15"
    );

    Ok(())
}

/// Test Case: Fill a gap and verify block access after archival.
/// Objective: Create a gap, fill it, wait for the archiver to process the
/// now-contiguous range, and then confirm that a block from that range can
// be read successfully (implying it was correctly moved to cold storage).
#[tokio::test]
#[serial]
async fn test_fill_gap_and_verify_archival() -> Result<()> {
    // Custom config with a small hot tier to force archival quickly.
    let custom_config = r#"
[core]
log_level = "info"
database_path = "/app/data/db"
start_block_number = 0

[plugins]
    [plugins.persistence_service]
    enabled = true
    cold_storage_path = "/app/data/cold"
    hot_storage_block_count = 10
    archive_batch_size = 5

    [plugins.observability]
    enabled = true
    listen_address = "0.0.0.0:8080"

    [plugins.verification_service]
    enabled = true

    [plugins.state_management_service]
    enabled = true

    [plugins.publish_service]
    enabled = true
    grpc_address = "0.0.0.0"
    grpc_port = 50051
    max_concurrent_streams = 10
    persistence_ack_timeout_seconds = 10
    stale_winner_timeout_seconds = 10
    winner_cleanup_interval_seconds = 10
    winner_cleanup_threshold_blocks = 100

    [plugins.subscriber_service]
    enabled = true
    grpc_address = "0.0.0.0"
    grpc_port = 50052
    max_concurrent_streams = 10
    live_stream_queue_size = 100
    max_future_block_lookahead = 100
    session_timeout_seconds = 10

    [plugins.block_access_service]
    enabled = true
    grpc_address = "0.0.0.0"
    grpc_port = 50053

    [plugins.server_status_service]
    enabled = true
    grpc_address = "0.0.0.0"
    grpc_port = 50054

    [plugins.query_service]
    enabled = true
    grpc_address = "0.0.0.0"
    grpc_port = 50055
"#;
    let ctx = TestContext::with_config(Some(custom_config), None).await?;

    // 1. Create a gap by publishing 0-4, then 10-14. Blocks 5-9 are missing.
    publish_blocks(&ctx, 0, 4).await?;
    publish_blocks(&ctx, 10, 14).await?;

    // 2. Fill the gap
    publish_blocks(&ctx, 5, 9).await?;

    // 3. Wait for the archiver to run.
    // The archiver now has a contiguous range from 0-14.
    // Given the config (batch size 5), it should run multiple times.
    println!("Waiting for archival cycles to complete...");
    tokio::time::sleep(Duration::from_secs(35)).await;

    // 4. Verify that an early block (which should now be archived) is accessible.
    let mut access_client = ctx.access_client().await?;
    let response = access_client
        .get_block(BlockRequest {
            block_specifier: Some(BlockSpecifier::BlockNumber(3)),
        })
        .await?
        .into_inner();

    assert_eq!(
        response.status,
        block_response::Code::Success as i32,
        "Should be able to access archived block #3 after the gap was filled"
    );
    let block = response.block.unwrap();
    let header = block.items.first().unwrap().item.as_ref().unwrap();
    if let rock_node_protobufs::com::hedera::hapi::block::stream::block_item::Item::BlockHeader(h) =
        header
    {
        assert_eq!(h.number, 3);
    } else {
        panic!("First item was not a block header!");
    }

    Ok(())
}
