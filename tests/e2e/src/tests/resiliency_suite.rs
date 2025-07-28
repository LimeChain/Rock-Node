use crate::common::{publish_blocks, TestContext};
use anyhow::Result;
use rock_node_protobufs::org::hiero::block::api::{
    block_request::BlockSpecifier, block_response, BlockRequest, ServerStatusRequest,
};
use serial_test::serial;
use std::process::Command;
use std::time::Duration;
use uuid::Uuid;

/// Test Case: Node Restart Retains State
/// Objective: Verify that a node can restart and retain its state.
#[tokio::test]
#[serial]
async fn test_node_restart_retains_state() -> Result<()> {
    let volume_name = format!("rock-node-test-volume-{}", Uuid::new_v4());

    // --- First Run ---
    {
        println!("Starting first run with volume: {}", &volume_name);
        let ctx = TestContext::with_config(None, Some(&volume_name)).await?;
        publish_blocks(&ctx, 0, 10).await?;

        // Verify state before shutdown
        let mut status_client = ctx.status_client().await?;
        let response = status_client
            .server_status(ServerStatusRequest::default())
            .await?
            .into_inner();
        assert_eq!(response.last_available_block, 10);

        println!("Stopping container: {}", ctx.container.id());
        ctx.container.stop().await?;
    }

    tokio::time::sleep(Duration::from_secs(2)).await;

    // --- Second Run (re-attaching to the same volume) ---
    {
        println!(
            "Starting second run, re-attaching to volume: {}",
            &volume_name
        );
        let ctx_restarted = TestContext::with_config(None, Some(&volume_name)).await?;

        let mut status_client = ctx_restarted.status_client().await?;
        let status_response = status_client
            .server_status(ServerStatusRequest::default())
            .await?
            .into_inner();

        assert_eq!(status_response.first_available_block, 0);
        assert_eq!(status_response.last_available_block, 10);

        let mut access_client = ctx_restarted.access_client().await?;
        let access_response = access_client
            .get_block(BlockRequest {
                block_specifier: Some(BlockSpecifier::BlockNumber(5)),
            })
            .await?
            .into_inner();
        assert_eq!(access_response.status, block_response::Code::Success as i32);

        publish_blocks(&ctx_restarted, 11, 11).await?;
        let final_status_response = status_client
            .server_status(ServerStatusRequest::default())
            .await?
            .into_inner();
        assert_eq!(final_status_response.last_available_block, 11);

        let _ = Command::new("docker")
            .args(["volume", "rm", "-f", &volume_name])
            .output();
    }

    Ok(())
}

/// Test Case: Hot to Cold Storage Archival
/// Objective: Verify that blocks are archived to cold storage after a certain number of blocks.
#[tokio::test]
#[serial]
async fn test_hot_to_cold_storage_archival() -> Result<()> {
    // Custom config with small, fast archival settings
    let custom_config = r#"
[core]
log_level = "info"
database_path = "/app/data/db"

[plugins]
    [plugins.observability]
    enabled = true
    listen_address = "0.0.0.0:8080"

    [plugins.persistence_service]
    enabled = true
    cold_storage_path = "/app/data/cold"
    hot_storage_block_count = 10
    archive_batch_size = 5

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

    // Publish enough blocks to trigger archival.
    // Hot tier holds 10. We publish 15, so 5 should be archived.
    publish_blocks(&ctx, 0, 14).await?;

    // The archiver runs on a 30s interval, so we wait.
    println!("Waiting for archival cycle to run...");
    tokio::time::sleep(Duration::from_secs(35)).await;

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
        "Should be able to access archived block #3"
    );

    let http_port = ctx.http_port().await?;
    let metrics_url = format!("http://localhost:{}/metrics", http_port);
    let metrics_body = reqwest::get(&metrics_url).await?.text().await?;

    let archival_cycle_line = metrics_body
        .lines()
        .find(|line| line.starts_with("rocknode_persistence_archival_cycles_total"))
        .expect("Metrics should contain archival cycle count");

    assert_eq!(
        archival_cycle_line.split_whitespace().last().unwrap_or("0"),
        "1",
        "Archival cycle count should be 1"
    );

    Ok(())
}
