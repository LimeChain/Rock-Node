use crate::common::{publish_blocks, TestContext};
use anyhow::Result;
use rock_node_protobufs::org::hiero::block::api::ServerStatusRequest;
use serial_test::serial;

/// Test Case: Server Status
///
/// Objective: Verify that the `server_status` endpoint correctly reports the
/// first and last available block numbers after data has been persisted.
#[tokio::test]
#[serial]
async fn test_server_status_returns_correct_block_range() -> Result<()> {
    let ctx = TestContext::new().await?;
    publish_blocks(&ctx, 0, 30).await?;
    let mut status_client = ctx.status_client().await?;

    let request = ServerStatusRequest::default();
    let response = status_client.server_status(request).await?.into_inner();

    assert_eq!(
        response.first_available_block, 0,
        "First available block is incorrect",
    );
    assert_eq!(
        response.last_available_block, 30,
        "Last available block is incorrect",
    );
    Ok(())
}
