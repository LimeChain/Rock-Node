use crate::common::block_builder::BlockBuilder;
use anyhow::Result;
use once_cell::sync::Lazy;
use prost::Message;
use rock_node_protobufs::org::hiero::block::api::block_stream_publish_service_client::BlockStreamPublishServiceClient;
use rock_node_protobufs::org::hiero::block::api::block_stream_subscribe_service_client::BlockStreamSubscribeServiceClient;
use rock_node_protobufs::org::hiero::block::api::{
    publish_stream_request::Request as PublishRequest, BlockItemSet, PublishStreamRequest,
};
use rock_node_protobufs::proto::network_service_client::NetworkServiceClient;
use rock_node_protobufs::proto::schedule_service_client::ScheduleServiceClient;
use rock_node_protobufs::proto::smart_contract_service_client::SmartContractServiceClient;
use rock_node_protobufs::proto::token_service_client::TokenServiceClient;
use rock_node_protobufs::proto::{
    consensus_service_client::ConsensusServiceClient, crypto_service_client::CryptoServiceClient,
    file_service_client::FileServiceClient,
};
use std::path::PathBuf;
use std::process::Command;
use testcontainers::runners::AsyncRunner;
use testcontainers::{
    core::{IntoContainerPort, Mount, WaitFor},
    ContainerAsync, CopyDataSource, GenericImage, ImageExt,
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
    /// running the `rock-node-e2e` image.
    pub async fn new() -> Result<TestContext> {
        Self::with_config(None, None).await
    }

    /// Creates a new `TestContext` with a specific configuration and optional
    /// named volume for data persistence across restarts.
    pub async fn with_config(
        config_override: Option<&str>,
        volume_name: Option<&str>,
    ) -> Result<TestContext> {
        Lazy::force(&E2E_IMAGE_BUILT);

        let config_content = match config_override {
            Some(content) => content.to_string(),
            None => {
                let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../config/config.e2e.toml")
                    .canonicalize()?;
                std::fs::read_to_string(&config_path)?
            }
        };

        let mut image = GenericImage::new("rock-node-e2e", "latest")
            .with_exposed_port(8080.tcp())
            .with_exposed_port(50051.tcp())
            .with_exposed_port(50052.tcp())
            .with_exposed_port(50053.tcp())
            .with_exposed_port(50054.tcp())
            .with_exposed_port(50055.tcp()) // Query service port
            .with_wait_for(WaitFor::message_on_stderr(
                "Rock Node running successfully!",
            ))
            .with_env_var("RUST_LOG", "info,rock_node_persistence_plugin=trace")
            .with_cmd(vec![
                "--config-path".to_string(),
                "/config/config.toml".to_string(),
            ])
            .with_copy_to(
                "/config/config.toml",
                CopyDataSource::Data(config_content.into_bytes()),
            );

        if let Some(vol_name) = volume_name {
            // Mount a named volume to persist data
            let mount = Mount::volume_mount(vol_name, "/app/data");
            image = image.with_mount(mount);
        }

        let container = image.start().await?;

        Ok(TestContext { container })
    }

    /// Get the HTTP port of the container for observability.
    pub async fn http_port(&self) -> Result<u16> {
        let port = self.container.get_host_port_ipv4(8080).await?;
        Ok(port)
    }

    pub async fn publisher_client(&self) -> Result<BlockStreamPublishServiceClient<Channel>> {
        let port = self.container.get_host_port_ipv4(50051).await?;
        let endpoint = format!("http://localhost:{}", port);
        let channel = Channel::from_shared(endpoint)?.connect().await?;
        Ok(BlockStreamPublishServiceClient::new(channel))
    }

    pub async fn subscriber_client_port(&self) -> Result<u16> {
        self.container
            .get_host_port_ipv4(50052)
            .await
            .map_err(Into::into)
    }

    pub async fn subscriber_client(&self) -> Result<BlockStreamSubscribeServiceClient<Channel>> {
        let port = self.container.get_host_port_ipv4(50052).await?;
        let endpoint = format!("http://localhost:{}", port);
        let channel = Channel::from_shared(endpoint)?.connect().await?;
        Ok(BlockStreamSubscribeServiceClient::new(channel))
    }

    pub async fn access_client(
        &self,
    ) -> Result<
        rock_node_protobufs::org::hiero::block::api::block_access_service_client::BlockAccessServiceClient<
            Channel,
        >,
    >{
        let port = self.container.get_host_port_ipv4(50053).await?;
        let endpoint = format!("http://localhost:{}", port);
        let channel = Channel::from_shared(endpoint)?.connect().await?;
        Ok(
            rock_node_protobufs::org::hiero::block::api::block_access_service_client::BlockAccessServiceClient::new(
                channel,
            ),
        )
    }

    pub async fn status_client(
        &self,
    ) -> Result<
        rock_node_protobufs::org::hiero::block::api::block_node_service_client::BlockNodeServiceClient<
            Channel,
        >,
    >{
        let port = self.container.get_host_port_ipv4(50054).await?;
        let endpoint = format!("http://localhost:{}", port);
        let channel = Channel::from_shared(endpoint)?.connect().await?;
        Ok(
            rock_node_protobufs::org::hiero::block::api::block_node_service_client::BlockNodeServiceClient::new(
                channel,
            ),
        )
    }

    pub async fn query_client(&self) -> Result<CryptoServiceClient<Channel>> {
        let port = self.container.get_host_port_ipv4(50055).await?;
        let endpoint = format!("http://localhost:{}", port);
        let channel = Channel::from_shared(endpoint)?.connect().await?;
        Ok(CryptoServiceClient::new(channel))
    }

    pub async fn consensus_client(&self) -> Result<ConsensusServiceClient<Channel>> {
        let port = self.container.get_host_port_ipv4(50055).await?;
        let endpoint = format!("http://localhost:{}", port);
        let channel = Channel::from_shared(endpoint)?.connect().await?;
        Ok(ConsensusServiceClient::new(channel))
    }

    pub async fn file_client(&self) -> Result<FileServiceClient<Channel>> {
        let port = self.container.get_host_port_ipv4(50055).await?;
        let endpoint = format!("http://localhost:{}", port);
        let channel = Channel::from_shared(endpoint)?.connect().await?;
        Ok(FileServiceClient::new(channel))
    }

    pub async fn network_client(&self) -> Result<NetworkServiceClient<Channel>> {
        let port = self.container.get_host_port_ipv4(50055).await?;
        let endpoint = format!("http://localhost:{}", port);
        let channel = Channel::from_shared(endpoint)?.connect().await?;
        Ok(NetworkServiceClient::new(channel))
    }

    pub async fn schedule_client(&self) -> Result<ScheduleServiceClient<Channel>> {
        let port = self.container.get_host_port_ipv4(50055).await?;
        let endpoint = format!("http://localhost:{}", port);
        let channel = Channel::from_shared(endpoint)?.connect().await?;
        Ok(ScheduleServiceClient::new(channel))
    }

    pub async fn token_client(&self) -> Result<TokenServiceClient<Channel>> {
        let port = self.container.get_host_port_ipv4(50055).await?;
        let endpoint = format!("http://localhost:{}", port);
        let channel = Channel::from_shared(endpoint)?.connect().await?;
        Ok(TokenServiceClient::new(channel))
    }

    pub async fn contract_client(&self) -> Result<SmartContractServiceClient<Channel>> {
        let port = self.container.get_host_port_ipv4(50055).await?;
        let endpoint = format!("http://localhost:{}", port);
        let channel = Channel::from_shared(endpoint)?.connect().await?;
        Ok(SmartContractServiceClient::new(channel))
    }
}

/// Publishes blocks in the inclusive range `[start, end]` using the node's publish service.
pub async fn publish_blocks(ctx: &TestContext, start: u64, end: u64) -> Result<()> {
    let mut client = ctx.publisher_client().await?;
    let (tx, rx) = mpsc::channel(10); // Use a buffered channel

    // We need a separate task for sending because the main task will be busy
    // waiting for server responses. This is the key to avoiding the deadlock.
    let sender_task = tokio::spawn(async move {
        for i in start..=end {
            let block_bytes = BlockBuilder::new(i).build();
            let block_proto: rock_node_protobufs::com::hedera::hapi::block::stream::Block =
                Message::decode(block_bytes.as_slice()).unwrap();
            let req = PublishStreamRequest {
                request: Some(PublishRequest::BlockItems(BlockItemSet {
                    block_items: block_proto.items,
                })),
            };
            if tx.send(req).await.is_err() {
                // This would happen if the receiver (the main part of this function)
                // is dropped, which means the test has already failed or finished.
                break;
            }
        }
        // When tx is dropped here, the stream will close on the server side.
    });

    // Now, create the stream and immediately start draining the responses.
    let mut responses = client
        .publish_block_stream(ReceiverStream::new(rx))
        .await?
        .into_inner();

    // Drain responses to ensure all blocks are acknowledged by the server.
    // The stream will end naturally when the sender_task completes and drops `tx`.
    while responses.next().await.is_some() {}

    // Ensure the sender task finished without panicking.
    sender_task.await?;

    Ok(())
}