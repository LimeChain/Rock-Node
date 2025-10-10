use crate::common::{block_builder::BlockBuilder, TestContext};
use anyhow::Result;
use prost::Message;
use rock_node_protobufs::org::hiero::block::api::{
    publish_stream_request::Request as PublishRequest,
    publish_stream_response::{BlockAcknowledgement, Response as PublishResponse},
    BlockItemSet, PublishStreamRequest,
};
use serial_test::serial;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, StreamExt};

/// Test Case: Happy Path – a single publisher sends a full block and receives an acknowledgement.
#[tokio::test]
#[serial]
async fn test_publish_single_block_successfully() -> Result<()> {
    let ctx = TestContext::new().await?;
    let mut client = ctx.publisher_client().await?;

    let (tx, rx) = mpsc::channel(8);
    let mut responses = client
        .publish_block_stream(ReceiverStream::new(rx))
        .await?
        .into_inner();

    // Build a complete block
    let block_bytes = BlockBuilder::new(0).build();
    let block_proto: rock_node_protobufs::com::hedera::hapi::block::stream::Block =
        Message::decode(block_bytes.as_slice())?;

    // Send the complete block
    tx.send(PublishStreamRequest {
        request: Some(PublishRequest::BlockItems(BlockItemSet {
            block_items: block_proto.items,
        })),
    })
    .await?;

    // Wait for acknowledgement
    let response = tokio::time::timeout(Duration::from_secs(5), responses.next())
        .await?
        .ok_or(anyhow::anyhow!("No response from server"))??;

    match response.response {
        Some(PublishResponse::Acknowledgement(ack)) => {
            assert_eq!(ack.block_number, 0, "Expected acknowledgement for block 0");
        },
        other => panic!("Expected Acknowledgement, got {:?}", other),
    }

    println!("✅ Single block published and acknowledged successfully");
    Ok(())
}
