use crate::common::{publish_blocks, TestContext};
use anyhow::Result;
use rock_node_protobufs::org::hiero::block::api::{
    block_request::BlockSpecifier, block_response, BlockRequest,
};
use serial_test::serial;
use std::time::Duration;
use testcontainers::core::Host;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, Mutex};
use tokio_stream::wrappers::TcpListenerStream;
use tonic::{transport::Server, Request, Response, Status};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use rock_node_protobufs::org::hiero::block::api::{
    block_stream_subscribe_service_server::{
        BlockStreamSubscribeService, BlockStreamSubscribeServiceServer,
    },
    subscribe_stream_response::Response as SubResponse, BlockItemSet, SubscribeStreamRequest,
    SubscribeStreamResponse, ServerStatusRequest, ServerStatusResponse,
    block_node_service_server::{BlockNodeService, BlockNodeServiceServer},
};
use tokio_stream::StreamExt;
use rock_node_protobufs::com::hedera::hapi::block::stream::{block_item, Block, BlockItem};
use prost::Message;


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
    async fn server_status(&self, _request: Request<ServerStatusRequest>) -> Result<Response<ServerStatusResponse>, Status> {
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

        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }
}

async fn spawn_mock_failing_peer(
    failures_to_simulate: usize,
    first: u64,
    last: u64,
) -> Result<(String, Arc<MockFailingPeerServer>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
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

    tokio::spawn(async move {
        Server::builder()
            .add_service(BlockNodeServiceServer::new(server_for_task.clone()))
            .add_service(BlockStreamSubscribeServiceServer::new(server_for_task))
            .serve_with_incoming(listener_stream)
            .await
    });

    Ok((format!("http://localhost:{}", addr.port()), server_clone))
}


/// Test Case: Backfill with a peer that fails and then succeeds.
/// Objective: Verify that the backfill plugin can handle an intermittent failure
/// by retrying with exponential backoff and eventually succeeding.
#[tokio::test]
#[serial("backfill_resilience")]
async fn test_backfill_with_intermittent_peer_failure() -> Result<()> {
    // --- 1. Setup a "source" node with a complete history and intermittent failure ---
    // Will fail the first 3 connection attempts, then succeed.
    let (failing_peer_address, server_handle) = spawn_mock_failing_peer(3, 10, 20).await?;
    
    // --- 2. Setup a "destination" node with a gap, configured to backfill from the failing peer ---
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
    check_interval_seconds = 1
"#,
        failing_peer_address
    );

    let dest_ctx = TestContext::with_config(Some(&dest_config), None).await?;

    // Create a gap (blocks 10-20 are missing)
    publish_blocks(&dest_ctx, 0, 9).await?;
    publish_blocks(&dest_ctx, 21, 30).await?;

    // --- 3. Wait for the backfill process to run ---
    println!("Waiting for GapFill cycle to complete, with expected failures...");
    tokio::time::sleep(Duration::from_secs(20)).await;

    // --- 4. Verify the gap has been filled ---
    let mut access_client = dest_ctx.access_client().await?;
    let response_after = access_client
        .get_block(BlockRequest {
            block_specifier: Some(BlockSpecifier::BlockNumber(15)),
        })
        .await?
        .into_inner();

    assert_eq!(
        response_after.status,
        block_response::Code::Success as i32,
        "Block 15 should be accessible after backfill"
    );

    // This test has to simulate all 3 retries before success.
    assert_eq!(server_handle.fail_count.load(Ordering::SeqCst), 3, "The mock server should have failed exactly three times before succeeding");

    Ok(())
}

/// Test Case: Backfill with a peer that is behind local.
/// Objective: Verify that the backfill plugin checks the peer's block range and correctly handles an unsuitable peer.
#[tokio::test]
#[serial("backfill_resilience")]
async fn test_backfill_handles_behind_peer() -> Result<()> {
    // --- 1. Setup a "source" node that is behind the required gap ---
    // The peer only has blocks up to 9, but the gap starts at 10.
    let (peer_address, _) = spawn_mock_failing_peer(0, 0, 9).await?;
    
    // --- 2. Setup a "destination" node with a gap, configured to backfill from the behind peer ---
    let dest_config = format!(
        r#"
[core]
log_level = "info"
database_path = "/app/data"
start_block_number = 0

[plugins]
    [plugins.persistence_service]
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

    // Create a gap (blocks 10-20 are missing)
    publish_blocks(&dest_ctx, 0, 9).await?;
    publish_blocks(&dest_ctx, 21, 30).await?;

    // --- 3. Wait for the backfill process to run once ---
    println!("Waiting for GapFill cycle to check the behind peer...");
    tokio::time::sleep(Duration::from_secs(10)).await;

    // --- 4. Verify the gap has NOT been filled ---
    let mut access_client = dest_ctx.access_client().await?;
    let response_after = access_client
        .get_block(BlockRequest {
            block_specifier: Some(BlockSpecifier::BlockNumber(15)),
        })
        .await?
        .into_inner();

    assert_eq!(
        response_after.status,
        block_response::Code::NotFound as i32,
        "Block 15 should NOT be accessible as the peer was behind"
    );

    // The test passes if the block is still not found, demonstrating that the client correctly identified the unsuitable peer and moved on.
    Ok(())
}

/// Test Case: Backfill with multiple peers, one of which works.
/// Objective: Verify that the backfill plugin can failover from one or more
/// offline peers and successfully connect to a working one.
#[tokio::test]
#[serial("backfill_resilience")]
async fn test_backfill_with_multiple_peers_failover() -> Result<()> {
    // --- 1. Setup multiple peers, some offline and one working ---
    let bad_peer_1 = "http://localhost:1".to_string(); // Guaranteed to fail
    let bad_peer_2 = "http://localhost:2".to_string(); // Guaranteed to fail
    let (good_peer_address, _) = spawn_mock_failing_peer(0, 0, 20).await?;

    let peers = vec![bad_peer_1, good_peer_address, bad_peer_2];

    // --- 2. Setup a "destination" node with a gap ---
    let dest_config = format!(
        r#"
[core]
log_level = "info"
database_path = "/app/data"
start_block_number = 0

[plugins]
    [plugins.persistence_service]
    enabled = true
    [plugins.backfill]
    enabled = true
    mode = "GapFill"
    peers = ["{}", "{}", "{}"]
    check_interval_seconds = 5
"#,
        peers[0], peers[1], peers[2]
    );

    let dest_ctx = TestContext::with_config(Some(&dest_config), None).await?;
    
    // Create a gap (blocks 10-20 are missing)
    publish_blocks(&dest_ctx, 0, 9).await?;

    // --- 3. Wait for the backfill to run ---
    println!("Waiting for backfill to failover and fill the gap...");
    tokio::time::sleep(Duration::from_secs(15)).await;

    // --- 4. Verify the gap has been filled ---
    let mut access_client = dest_ctx.access_client().await?;
    let response_after = access_client
        .get_block(BlockRequest {
            block_specifier: Some(BlockSpecifier::BlockNumber(15)),
        })
        .await?
        .into_inner();

    assert_eq!(
        response_after.status,
        block_response::Code::Success as i32,
        "Block 15 should be accessible after backfill"
    );

    Ok(())
}
