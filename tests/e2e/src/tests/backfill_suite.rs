use std::time::Duration;

use crate::common::{publish_blocks, TestContext};
use anyhow::Result;
use rock_node_protobufs::org::hiero::block::api::{
    block_request::BlockSpecifier, block_response, BlockRequest,
};
use serial_test::serial;

/// Test Case: GapFill Mode
/// Objective: Verify that a node with a gap in its block history can fill it from a peer.
#[tokio::test]
#[serial]
async fn test_gap_fill_mode_successfully_fills_gap() -> Result<()> {
    // --- 1. Setup a "source" node with a complete history ---
    let source_ctx = TestContext::new().await?;
    publish_blocks(&source_ctx, 0, 20).await?;

    let subscriber_port = source_ctx.subscriber_client_port().await?;
    let peer_address = format!("http://localhost:{}", subscriber_port);

    // --- 2. Setup a "destination" node with a gap ---
    // This node will be configured to backfill from the source node.
    let dest_config = format!(
        r#"
[core]
log_level = "info"
database_path = "/app/data"
start_block_number = 0

[plugins]
    [plugins.persistence_service]
    enabled = true
    [plugins.publish_service]
    enabled = true
    grpc_port = 50051
    [plugins.block_access_service]
    enabled = true
    grpc_port = 50053
    [plugins.subscriber_service]
    enabled = true
    grpc_port = 50052
    [plugins.backfill]
    enabled = true
    mode = "GapFill"
    peers = ["{}"]
    check_interval_seconds = 5
"#,
        peer_address
    );

    let dest_ctx = TestContext::with_config(Some(&dest_config), None).await?;

    // Publish blocks with a gap (0-9, then 16-20, leaving 10-15 missing)
    publish_blocks(&dest_ctx, 0, 9).await?;
    publish_blocks(&dest_ctx, 16, 20).await?;

    // Verify the gap exists before backfill
    let mut access_client = dest_ctx.access_client().await?;
    let response_before = access_client
        .get_block(BlockRequest {
            block_specifier: Some(BlockSpecifier::BlockNumber(12)),
        })
        .await?
        .into_inner();

    assert_eq!(
        response_before.status,
        block_response::Code::NotFound as i32,
        "Block 12 should not be found before backfill"
    );

    // --- 3. Wait for the backfill process to run ---
    println!("Waiting for GapFill cycle to complete...");
    tokio::time::sleep(Duration::from_secs(10)).await;

    // --- 4. Verify the gap has been filled ---
    let response_after = access_client
        .get_block(BlockRequest {
            block_specifier: Some(BlockSpecifier::BlockNumber(12)),
        })
        .await?
        .into_inner();

    assert_eq!(
        response_after.status,
        block_response::Code::Success as i32,
        "Block 12 should be accessible after backfill"
    );

    let block = response_after
        .block
        .expect("Response should contain a block");
    let header = block
        .items
        .iter()
        .find_map(|item| match &item.item {
            Some(
                rock_node_protobufs::com::hedera::hapi::block::stream::block_item::Item::BlockHeader(
                    h,
                ),
            ) => Some(h),
            _ => None,
        })
        .expect("Returned block must have a header");

    assert_eq!(
        header.number, 12,
        "The correct block should have been backfilled"
    );

    Ok(())
}

/// Test Case: Continuous Mode
/// Objective: Verify that a node can catch up to a peer and then continue streaming live blocks.
#[tokio::test]
#[serial]
async fn test_continuous_mode_successfully_catches_up_and_streams() -> Result<()> {
    // --- 1. Setup a "source" node and publish some initial blocks ---
    let source_ctx = TestContext::new().await?;
    publish_blocks(&source_ctx, 0, 10).await?;

    let subscriber_port = source_ctx.subscriber_client_port().await?;
    let peer_address = format!("http://localhost:{}", subscriber_port);

    // --- 2. Setup a new, empty "destination" node in Continuous mode ---
    let dest_config = format!(
        r#"
[core]
log_level = "info"
database_path = "/app/data"
start_block_number = 0

[plugins]
    [plugins.persistence_service]
    enabled = true
    [plugins.publish_service]
    enabled = true
    grpc_port = 50051
    [plugins.block_access_service]
    enabled = true
    grpc_port = 50053
    [plugins.subscriber_service]
    enabled = true
    grpc_port = 50052
    [plugins.backfill]
    enabled = true
    mode = "Continuous"
    peers = ["{}"]
"#,
        peer_address
    );
    let dest_ctx = TestContext::with_config(Some(&dest_config), None).await?;

    // --- 3. Wait for the initial catch-up ---
    println!("Waiting for Continuous mode to catch up...");
    tokio::time::sleep(Duration::from_secs(10)).await;

    // --- 4. Verify the initial set of blocks was backfilled ---
    let mut access_client = dest_ctx.access_client().await?;
    for i in 0..=10 {
        let response = access_client
            .get_block(BlockRequest {
                block_specifier: Some(BlockSpecifier::BlockNumber(i)),
            })
            .await?
            .into_inner();
        assert_eq!(
            response.status,
            block_response::Code::Success as i32,
            "Block {} should be accessible after initial catch-up",
            i
        );
    }

    // --- 5. Publish new blocks to the source node ---
    println!("Publishing new blocks to source node...");
    publish_blocks(&source_ctx, 11, 15).await?;

    // --- 6. Wait for the continuous stream to deliver the new blocks ---
    tokio::time::sleep(Duration::from_secs(5)).await;

    // --- 7. Verify the new blocks have been streamed ---
    for i in 11..=15 {
        let response = access_client
            .get_block(BlockRequest {
                block_specifier: Some(BlockSpecifier::BlockNumber(i)),
            })
            .await?
            .into_inner();
        assert_eq!(
            response.status,
            block_response::Code::Success as i32,
            "Block {} should be accessible after live streaming",
            i
        );
    }

    Ok(())
}
