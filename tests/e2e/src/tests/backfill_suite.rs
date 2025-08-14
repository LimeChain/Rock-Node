use crate::common::{publish_blocks, TestContext};
use anyhow::Result;
use rock_node_protobufs::org::hiero::block::api::ServerStatusRequest;
use serial_test::serial;
use std::time::Duration;

/// Test Case: GapFill Mode
/// Objective: Verify that the backfill plugin can detect a gap and fill it
/// by requesting the missing blocks from a peer.
#[tokio::test]
#[serial]
async fn test_backfill_gap_fill_mode() -> Result<()> {
    // 1. Setup Peer Node
    let peer_ctx = TestContext::new().await?;
    // The peer will have a complete history
    publish_blocks(&peer_ctx, 0, 20).await?;

    // 2. Setup Target Node with a custom config pointing to the Peer
    let peer_port = peer_ctx.subscriber_client_port().await?;
    let target_config = format!(
        r#"
[core]
log_level = "info"
database_path = "/app/data"
start_block_number = 0
[plugins.publish_service]
enabled = true
grpc_port = 50051
[plugins.subscriber_service]
enabled = true
grpc_port = 50052
[plugins.backfill]
enabled = true
mode = "GapFill"
peers = ["http://host.testcontainers.internal:{}"]
check_interval_seconds = 5
max_batch_size = 10
"#,
        peer_port
    );

    let target_ctx = TestContext::with_config(Some(&target_config), None).await?;

    // 3. Create a gap on the Target Node
    publish_blocks(&target_ctx, 0, 5).await?;
    publish_blocks(&target_ctx, 11, 15).await?;

    // Verify the gap exists
    let mut status_client = target_ctx.status_client().await?;
    let initial_status = status_client.server_status(ServerStatusRequest::default()).await?.into_inner();
    assert_eq!(initial_status.last_available_block, 15);
    // We can't directly check for gaps via API, but we know it's there.

    // 4. Wait for the backfill plugin to run and fill the gap
    println!("Waiting for GapFill to run...");
    tokio::time::sleep(Duration::from_secs(10)).await;

    // 5. Verify the gap is filled
    let final_status = status_client.server_status(ServerStatusRequest::default()).await?.into_inner();
    // The highest contiguous should now have advanced past the gap
    // This isn't directly exposed by status, so we check for a block inside the gap
    let mut access_client = target_ctx.access_client().await?;
    let res = access_client.get_block(rock_node_protobufs::org::hiero::block::api::BlockRequest {
        block_specifier: Some(rock_node_protobufs::org::hiero::block::api::block_request::BlockSpecifier::BlockNumber(8)),
    }).await?.into_inner();

    assert_eq!(res.status, rock_node_protobufs::org::hiero::block::api::block_response::Code::Success as i32, "Block #8 should now be available after GapFill");

    Ok(())
}

/// Test Case: Continuous Mode
/// Objective: Verify a Tier 2 node can start from scratch and catch up to a peer.
#[tokio::test]
#[serial]
async fn test_backfill_continuous_mode() -> Result<()> {
    // 1. Setup Peer Node with existing history
    let peer_ctx = TestContext::new().await?;
    publish_blocks(&peer_ctx, 0, 25).await?;

    // 2. Setup Target Node (Tier 2) to backfill from the peer
    let peer_port = peer_ctx.subscriber_client_port().await?;
    let target_config = format!(
        r#"
[core]
log_level = "info"
database_path = "/app/data"
start_block_number = 0
[plugins.publish_service]
enabled = false # This is a Tier 2 node
[plugins.subscriber_service]
enabled = true
grpc_port = 50052
[plugins.backfill]
enabled = true
mode = "Continuous"
peers = ["http://host.testcontainers.internal:{}"]
check_interval_seconds = 5
max_batch_size = 10
"#,
        peer_port
    );

    let target_ctx = TestContext::with_config(Some(&target_config), None).await?;

    // 3. Wait for the node to catch up
    println!("Waiting for Continuous backfill to catch up...");
    tokio::time::sleep(Duration::from_secs(15)).await;

    // 4. Verify it's caught up
    let mut status_client = target_ctx.status_client().await?;
    let status = status_client.server_status(ServerStatusRequest::default()).await?.into_inner();
    assert_eq!(status.last_available_block, 25, "Tier 2 node should have backfilled all blocks from peer");

    // 5. Publish more blocks to the peer and see if the T2 node gets them live
    publish_blocks(&peer_ctx, 26, 30).await?;
    tokio::time::sleep(Duration::from_secs(10)).await;

    let final_status = status_client.server_status(ServerStatusRequest::default()).await?.into_inner();
    assert_eq!(final_status.last_available_block, 30, "Tier 2 node should have received live blocks after catching up");

    Ok(())
}