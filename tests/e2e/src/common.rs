use crate::common::block_builder::BlockBuilder;
use anyhow::Result;
use once_cell::sync::Lazy;
use prost::Message;
use rock_node_protobufs::org::hiero::block::api::block_stream_publish_service_client::BlockStreamPublishServiceClient;
use rock_node_protobufs::org::hiero::block::api::block_stream_subscribe_service_client::BlockStreamSubscribeServiceClient;
use rock_node_protobufs::org::hiero::block::api::{
    publish_stream_request::Request as PublishRequest, BlockItemSet, PublishStreamRequest,
};
use std::path::PathBuf;
use std::process::Command;
use testcontainers::runners::AsyncRunner;
use testcontainers::ContainerAsync;
use testcontainers::{
    core::{IntoContainerPort, WaitFor},
    CopyDataSource, GenericImage, ImageExt,
};
use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, StreamExt};
use tonic::transport::Channel;

static E2E_IMAGE_BUILT: Lazy<()> = Lazy::new(|| {
    // Build the rock-node-e2e image once for the whole test binary.
    let inspect = Command::new("docker")
        .args(["image", "inspect", "rock-node-e2e:latest"])
        .output()
        .expect("failed to execute docker image inspect");

    if inspect.status.success() {
        println!("Docker image 'rock-node-e2e:latest' exists, skipping build.");
        return;
    }

    println!("Building Docker image for E2E tests (one-time)…");
    let status = Command::new("docker")
        .args([
            "build",
            "-t",
            "rock-node-e2e:latest",
            "-f",
            "Dockerfile.e2e",
            "../..",
        ])
        .status()
        .expect("failed to run docker build");

    assert!(status.success(), "Failed to build rock-node-e2e image");
    println!("Docker image built successfully.");
});

pub mod block_builder;

pub struct TestContext {
    pub container: ContainerAsync<GenericImage>,
}

impl TestContext {
    /// Creates a new `TestContext` by starting an isolated Docker container
    /// running the `rock-node-e2e` image. This method:
    ///
    /// - Loads the E2E test configuration from `config/config.e2e.toml`.
    /// - Generates a unique container name for test isolation.
    /// - Starts a container with the configuration file mounted inside.
    /// - Waits for the node to signal readiness via a specific stdout message.
    ///
    /// Returns a `TestContext` containing the running container, which can be
    /// used to interact with the test node (e.g., to retrieve its HTTP port).
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration file cannot be read, the container
    /// fails to start, or any required Docker operation fails.
    pub async fn new() -> Result<TestContext> {
        // Ensure the image is built exactly once.
        Lazy::force(&E2E_IMAGE_BUILT);

        let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../config/config.e2e.toml")
            .canonicalize()?;

        // Read the config file content
        let config_content = std::fs::read_to_string(&config_path)?;

        // Generate unique container name for isolation (Docker-friendly format)
        let thread_id = format!("{:?}", std::thread::current().id());
        let thread_id_clean = thread_id.replace("ThreadId(", "").replace(")", "");
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let container_name = format!("rock-node-test-{}-{}", thread_id_clean, timestamp);

        let container = GenericImage::new("rock-node-e2e", "latest")
            .with_exposed_port(8080.tcp())
            .with_exposed_port(50051.tcp())
            .with_exposed_port(50052.tcp())
            .with_wait_for(WaitFor::message_on_stdout(
                "Rock Node running successfully.",
            ))
            .with_copy_to(
                "/config/config.toml",
                CopyDataSource::Data(config_content.into_bytes()),
            )
            .with_cmd(vec![
                "--config-path".to_string(),
                "/config/config.toml".to_string(),
            ])
            .with_container_name(&container_name)
            .start()
            .await?;

        Ok(TestContext { container })
    }

    /// Get the HTTP port of the container.
    pub async fn http_port(&self) -> Result<u16> {
        let port = self.container.get_host_port_ipv4(8080).await?;
        Ok(port)
    }

    /// Returns a gRPC client for the `BlockStreamPublishService` running inside the
    /// Rock-Node container.
    ///
    /// The port (50051) is defined in `config.e2e.toml` and mapped to a free host
    /// port by `testcontainers`.  We look up that mapping and create a tonic
    /// channel to it.
    pub async fn publisher_client(&self) -> Result<BlockStreamPublishServiceClient<Channel>> {
        // Resolve the mapped host port for container port 50051.
        let port = self.container.get_host_port_ipv4(50051).await?;
        let endpoint = format!("http://localhost:{}", port);

        let channel = Channel::from_shared(endpoint)?.connect().await?;

        Ok(BlockStreamPublishServiceClient::new(channel))
    }

    // SECOND_EDIT: add subscriber_client helper
    /// Returns a gRPC client for the `BlockStreamSubscribeService` running inside the
    /// Rock-Node container.
    pub async fn subscriber_client(&self) -> Result<BlockStreamSubscribeServiceClient<Channel>> {
        let port = self.container.get_host_port_ipv4(50052).await?;
        let endpoint = format!("http://localhost:{}", port);
        let channel = Channel::from_shared(endpoint)?.connect().await?;
        Ok(BlockStreamSubscribeServiceClient::new(channel))
    }
}

/// Publishes blocks in the inclusive range `[start, end]` using the node's publish service.
/// The function waits for the server to acknowledge each block before proceeding to the next.
pub async fn publish_blocks(ctx: &TestContext, start: u64, end: u64) -> Result<()> {
    let mut client = ctx.publisher_client().await?;

    for i in start..=end {
        let block_bytes = BlockBuilder::new(i).build();
        let block_proto: rock_node_protobufs::com::hedera::hapi::block::stream::Block =
            Message::decode(block_bytes.as_slice())?;

        let (tx, rx) = mpsc::channel(1);
        tx.send(PublishStreamRequest {
            request: Some(PublishRequest::BlockItems(BlockItemSet {
                block_items: block_proto.items,
            })),
        })
        .await?;
        drop(tx);

        let mut responses = client
            .publish_block_stream(ReceiverStream::new(rx))
            .await?
            .into_inner();
        while responses.next().await.is_some() {}
    }

    Ok(())
}
