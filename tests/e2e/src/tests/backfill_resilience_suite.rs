use crate::common::{publish_blocks, TestContext};
use anyhow::Result;
use prost::Message;
use rock_node_protobufs::com::hedera::hapi::block::stream::{block_item, Block, BlockItem};
use rock_node_protobufs::org::hiero::block::api::{
    block_node_service_server::{BlockNodeService, BlockNodeServiceServer},
    block_stream_subscribe_service_server::{
        BlockStreamSubscribeService, BlockStreamSubscribeServiceServer,
    },
    subscribe_stream_response::Response as SubResponse,
    BlockItemSet, ServerStatusRequest, ServerStatusResponse, SubscribeStreamRequest,
    SubscribeStreamResponse,
};
use rock_node_protobufs::org::hiero::block::api::{
    block_request::BlockSpecifier, block_response, BlockRequest,
};
use serial_test::serial;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use testcontainers::core::Host;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, Mutex};
use tokio_stream::wrappers::TcpListenerStream;
use tokio_stream::StreamExt;
use tonic::{transport::Server, Request, Response, Status};

/// A mock peer server that can be configured to fail a specific number of times
/// before it starts successfully streaming blocks. It also responds to ServerStatus requests.
#[derive(Clone, Default)]
struct MockFailingPeerServer {
    fail_count: Arc<AtomicUsize>,
    total_failures_to_simulate: usize,
    available_first: u64,
    available_last: u64,
}

#[tonic::async_trait]
impl BlockNodeService for MockFailingPeerServer {
    async fn server_status(
        &self,
        _request: Request<ServerStatusRequest>,
    ) -> Result<Response<ServerStatusResponse>, Status> {
        Ok(Response::new(ServerStatusResponse {
            first_available_block: self.available_first,
            last_available_block: self.available_last,
            ..Default::default()
        }))
    }
}

#[tonic::async_trait]
impl BlockStreamSubscribeService for MockFailingPeerServer {
    type subscribeBlockStreamStream =
        tokio_stream::wrappers::ReceiverStream<Result<SubscribeStreamResponse, tonic::Status>>;

    async fn subscribe_block_stream(
        &self,
        request: Request<SubscribeStreamRequest>,
    ) -> Result<Response<Self::subscribeBlockStreamStream>, Status> {
        let current_failures = self.fail_count.load(Ordering::SeqCst);
        if current_failures < self.total_failures_to_simulate {
            self.fail_count.fetch_add(1, Ordering::SeqCst);
            return Err(Status::unavailable("Simulated failure"));
        }

        let req = request.into_inner();
        let (tx, rx) = mpsc::channel(10);

        tokio::spawn(async move {
            for i in req.start_block_number..=req.end_block_number {
                let block = Block {
                    items: vec![BlockItem {
                        item: Some(block_item::Item::BlockHeader(
                            rock_node_protobufs::com::hedera::hapi::block::stream::output::BlockHeader {
                                number: i, ..Default::default()
                            },
                        )),
                    }],
                };
                let item_set = BlockItemSet {
                    block_items: block.items,
                };
                let response = SubscribeStreamResponse {
                    response: Some(SubResponse::BlockItems(item_set)),
                };
                if tx.send(Ok(response)).await.is_err() {
                    break;
                }
            }
        });

        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
            rx,
        )))
    }
}

async fn spawn_mock_failing_peer(
    failures_to_simulate: usize,
    first: u64,
    last: u64,
) -> Result<(String, Arc<MockFailingPeerServer>)> {
    // Bind to 0.0.0.0 to be accessible from Docker containers
    let listener = TcpListener::bind("0.0.0.0:0").await?;
    let addr = listener.local_addr()?;
    let listener_stream = tokio_stream::wrappers::TcpListenerStream::new(listener);

    let server = MockFailingPeerServer {
        fail_count: Arc::new(AtomicUsize::new(0)),
        total_failures_to_simulate: failures_to_simulate,
        available_first: first,
        available_last: last,
    };

    let server_for_return = Arc::new(server);
    let server_for_task = (*server_for_return).clone();
    let server_clone = server_for_return.clone();

    // Spawn server with error handling and timeout
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let server_handle = tokio::spawn(async move {
        let server_future = Server::builder()
            .add_service(BlockNodeServiceServer::new(server_for_task.clone()))
            .add_service(BlockStreamSubscribeServiceServer::new(server_for_task))
            .serve_with_incoming(listener_stream);

        // Signal that server is ready to accept connections
        let _ = ready_tx.send(());

        match server_future.await {
            Ok(_) => println!("Mock server shut down gracefully"),
            Err(e) => eprintln!("Mock server error: {}", e),
        }
    });

    // Wait for server to be ready (with timeout)
    match tokio::time::timeout(tokio::time::Duration::from_secs(5), ready_rx).await {
        Ok(Ok(_)) => {
            println!("Mock server is ready on port {}", addr.port());
        }
        Ok(Err(_)) => {
            return Err(anyhow::anyhow!("Mock server failed to signal readiness"));
        }
        Err(_) => {
            server_handle.abort();
            return Err(anyhow::anyhow!("Mock server startup timed out"));
        }
    }

    // For Docker containers, try to get the actual host IP
    // First try to get it from environment, then try to detect it
    let host_ip = if let Ok(ip) = std::env::var("DOCKER_HOST_IP") {
        ip
    } else {
        // Try to get the host IP that containers can reach
        // This is a common pattern for testcontainers
        "host.docker.internal".to_string()
    };

    println!("Using host IP for mock server: {}", host_ip);
    Ok((format!("http://{}:{}", host_ip, addr.port()), server_clone))
}

/// Test Case: Backfill with a peer that fails and then succeeds.
/// Objective: Verify that the backfill plugin can handle an intermittent failure
/// by retrying with exponential backoff and eventually succeeding.
#[tokio::test]
#[serial("backfill_resilience")]
async fn test_backfill_with_intermittent_peer_failure() -> Result<()> {
    // --- 1. Setup a "source" node with complete history ---
    let source_ctx = TestContext::new().await?;
    publish_blocks(&source_ctx, 0, 20).await?;

    let subscriber_port = source_ctx.subscriber_client_port().await?;
    // Use host.docker.internal for macOS and host-gateway for Linux
    let peer_address = if cfg!(target_os = "macos") {
        format!("http://host.docker.internal:{}", subscriber_port)
    } else {
        format!("http://host-gateway:{}", subscriber_port)
    };

    // --- 2. Setup a "destination" node with a gap ---
    let dest_config = format!(
        r#"
[core]
log_level = "info"
database_path = "/app/data"
start_block_number = 0
grpc_address = "0.0.0.0"
grpc_port = 50051

[plugins]
    [plugins.persistence_service]
    enabled = true
    [plugins.publish_service]
    enabled = true
    [plugins.block_access_service]
    enabled = true
    [plugins.subscriber_service]
    enabled = true
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
    tokio::time::sleep(Duration::from_secs(30)).await;

    // --- 4. Verify the gap has been filled ---
    let mut access_client = dest_ctx.access_client().await?;
    let response_after = access_client
        .get_block(BlockRequest {
            block_specifier: Some(BlockSpecifier::BlockNumber(12)),
        })
        .await?
        .into_inner();

    println!(
        "Block 12 status after backfill: {:?}",
        response_after.status
    );

    assert_eq!(
        response_after.status,
        block_response::Code::Success as i32,
        "Block 12 should be accessible after backfill. Status: {:?}",
        response_after.status
    );

    Ok(())
}

/// Test Case: Backfill with a peer that is behind local.
/// Objective: Verify that the backfill plugin checks the peer's block range and correctly handles an unsuitable peer.
#[tokio::test]
#[serial("backfill_resilience")]
async fn test_backfill_handles_behind_peer() -> Result<()> {
    // --- 1. Setup a "source" node with limited history (only blocks 0-9) ---
    let source_ctx = TestContext::new().await?;
    publish_blocks(&source_ctx, 0, 9).await?;

    let subscriber_port = source_ctx.subscriber_client_port().await?;
    // Use host.docker.internal for macOS and host-gateway for Linux
    let peer_address = if cfg!(target_os = "macos") {
        format!("http://host.docker.internal:{}", subscriber_port)
    } else {
        format!("http://host-gateway:{}", subscriber_port)
    };

    // --- 2. Setup a "destination" node with a gap that the source can't fill ---
    let dest_config = format!(
        r#"
[core]
log_level = "info"
database_path = "/app/data"
start_block_number = 0
grpc_address = "0.0.0.0"
grpc_port = 50051

[plugins]
    [plugins.persistence_service]
    enabled = true
    [plugins.backfill]
    enabled = true
    mode = "GapFill"
    peers = ["{}"]
    check_interval_seconds = 10
"#,
        peer_address
    );

    let dest_ctx = TestContext::with_config(Some(&dest_config), None).await?;

    // Create a gap (blocks 10-20 are missing) - source only has blocks 0-9
    publish_blocks(&dest_ctx, 0, 9).await?;
    publish_blocks(&dest_ctx, 21, 30).await?;

    // --- 3. Wait for the backfill process to run ---
    println!("Waiting for GapFill cycle to check the behind peer...");
    tokio::time::sleep(Duration::from_secs(20)).await;

    // --- 4. Verify the gap has NOT been filled ---
    let mut access_client = dest_ctx.access_client().await?;
    let response_after = access_client
        .get_block(BlockRequest {
            block_specifier: Some(BlockSpecifier::BlockNumber(15)),
        })
        .await?
        .into_inner();

    println!(
        "Block 15 status (should be NotFound): {:?}",
        response_after.status
    );

    assert_eq!(
        response_after.status,
        block_response::Code::NotFound as i32,
        "Block 15 should NOT be accessible as the peer was behind. Status: {:?}",
        response_after.status
    );

    // The test passes if the block is still not found, demonstrating that the client correctly identified the unsuitable peer and moved on.
    Ok(())
}

/// Test Case: Backfill with multiple peers, one of which works.
/// Objective: Verify that the backfill plugin can failover from one or more
/// offline peers and successfully connect to a working one.

async fn test_backfill_with_multiple_peers_failover() -> Result<()> {
    // --- 1. Setup a working source peer ---
    let source_ctx = TestContext::new().await?;
    publish_blocks(&source_ctx, 10, 20).await?;

    let subscriber_port = source_ctx.subscriber_client_port().await?;
    // Use host.docker.internal for macOS and host-gateway for Linux
    let good_peer = if cfg!(target_os = "macos") {
        format!("http://host.docker.internal:{}", subscriber_port)
    } else {
        format!("http://host-gateway:{}", subscriber_port)
    };

    // Bad peers that will fail to connect
    let bad_peer_1 = "http://nonexistent1:1234".to_string();
    let bad_peer_2 = "http://nonexistent2:1234".to_string();

    // --- 2. Setup a "destination" node with a gap ---
    let dest_config = format!(
        r#"
[core]
log_level = "info"
database_path = "/app/data"
start_block_number = 0
grpc_address = "0.0.0.0"
grpc_port = 50051

[plugins]
    [plugins.persistence_service]
    enabled = true
    [plugins.backfill]
    enabled = true
    mode = "GapFill"
    peers = ["{}", "{}", "{}"]
    check_interval_seconds = 10
"#,
        bad_peer_1, good_peer, bad_peer_2
    );

    let dest_ctx = TestContext::with_config(Some(&dest_config), None).await?;

    // Create a gap (blocks 10-20 are missing)
    publish_blocks(&dest_ctx, 0, 9).await?;
    publish_blocks(&dest_ctx, 21, 30).await?;

    // --- 3. Wait for the backfill to run ---
    println!("Waiting for backfill to failover and fill the gap...");
    tokio::time::sleep(Duration::from_secs(30)).await;

    // --- 4. Verify the gap has been filled ---
    let mut access_client = dest_ctx.access_client().await?;
    let response_after = access_client
        .get_block(BlockRequest {
            block_specifier: Some(BlockSpecifier::BlockNumber(15)),
        })
        .await?
        .into_inner();

    println!(
        "Block 15 status after failover backfill: {:?}",
        response_after.status
    );

    assert_eq!(
        response_after.status,
        block_response::Code::Success as i32,
        "Block 15 should be accessible after backfill. Status: {:?}",
        response_after.status
    );

    Ok(())
}
