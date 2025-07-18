use crate::common::{block_builder::BlockBuilder, TestContext};
use anyhow::Result;
use prost::Message;
use rock_node_protobufs::org::hiero::block::api::{
    publish_stream_request::Request as PublishRequest,
    publish_stream_response::{end_of_stream::Code as EndCode, Response as PublishResponse},
    BlockItemSet, PublishStreamRequest,
};
use serial_test::serial;
use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, StreamExt};

/// Test Case: Happy Path – a single publisher sends a full block and receives an acknowledgement.
#[tokio::test]
#[serial]
async fn test_publish_single_block_successfully() -> Result<()> {
    let ctx = TestContext::new().await?;
    let mut client = ctx.publisher_client().await?;

    let block_bytes = BlockBuilder::new(0).build();
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

    let response_stream = client.publish_block_stream(ReceiverStream::new(rx)).await?;
    let mut responses = response_stream.into_inner();

    let response = responses
        .next()
        .await
        .expect("Server should have sent a response")?;

    match response.response {
        Some(PublishResponse::Acknowledgement(ack)) => {
            assert_eq!(
                ack.block_number, 0,
                "Acknowledgement has wrong block number"
            );
        }
        other => panic!("Expected BlockAcknowledgement, got {:?}", other),
    }

    assert!(responses.next().await.is_none());
    Ok(())
}

/// Test Case: Race condition – two publishers send the same header concurrently.
#[tokio::test]
#[serial]
async fn test_publisher_race_condition() -> Result<()> {
    let ctx = TestContext::new().await?;
    let mut primary_client = ctx.publisher_client().await?;
    let mut secondary_client = ctx.publisher_client().await?;

    let header_only = {
        let block_bytes = BlockBuilder::new(0).build();
        let block_proto: rock_node_protobufs::com::hedera::hapi::block::stream::Block =
            Message::decode(block_bytes.as_slice())?;
        vec![block_proto.items[0].clone()]
    };

    let header_request = PublishStreamRequest {
        request: Some(PublishRequest::BlockItems(BlockItemSet {
            block_items: header_only,
        })),
    };

    let (primary_tx, primary_rx) = mpsc::channel(4);
    let (secondary_tx, secondary_rx) = mpsc::channel(4);

    primary_tx.send(header_request.clone()).await?;
    let mut primary_responses = primary_client
        .publish_block_stream(ReceiverStream::new(primary_rx))
        .await?
        .into_inner();

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    secondary_tx.send(header_request).await?;
    let mut secondary_responses = secondary_client
        .publish_block_stream(ReceiverStream::new(secondary_rx))
        .await?
        .into_inner();

    let secondary_response = secondary_responses
        .next()
        .await
        .expect("Secondary should receive response")?;
    match secondary_response.response {
        Some(PublishResponse::SkipBlock(skip)) => {
            assert_eq!(skip.block_number, 0);
        }
        other => panic!("Secondary expected SkipBlock, got {:?}", other),
    }

    // Primary should have no message yet (within 200 ms)
    let primary_result = tokio::time::timeout(
        std::time::Duration::from_millis(200),
        primary_responses.next(),
    )
    .await;
    assert!(
        primary_result.is_err(),
        "Primary unexpectedly received a message"
    );

    Ok(())
}

/// Test Case: Duplicate block – sending an already persisted block returns EndStream DuplicateBlock.
#[tokio::test]
#[serial]
async fn test_duplicate_block_rejected() -> Result<()> {
    let ctx = TestContext::new().await?;
    let mut client = ctx.publisher_client().await?;

    let block_bytes = BlockBuilder::new(0).build();
    let block_proto: rock_node_protobufs::com::hedera::hapi::block::stream::Block =
        Message::decode(block_bytes.as_slice())?;
    let (tx1, rx1) = mpsc::channel(1);
    tx1.send(PublishStreamRequest {
        request: Some(PublishRequest::BlockItems(BlockItemSet {
            block_items: block_proto.items.clone(),
        })),
    })
    .await?;
    drop(tx1);

    let mut responses = client
        .publish_block_stream(ReceiverStream::new(rx1))
        .await?
        .into_inner();
    while let Some(resp) = responses.next().await {
        let resp = resp?;
        if matches!(resp.response, Some(PublishResponse::Acknowledgement(_))) {
            break;
        }
    }

    let mut client2 = ctx.publisher_client().await?;
    let header_only = vec![block_proto.items[0].clone()];
    let (tx2, rx2) = mpsc::channel(1);
    tx2.send(PublishStreamRequest {
        request: Some(PublishRequest::BlockItems(BlockItemSet {
            block_items: header_only,
        })),
    })
    .await?;
    drop(tx2);

    let mut dup_resp_stream = client2
        .publish_block_stream(ReceiverStream::new(rx2))
        .await?
        .into_inner();

    let first_resp = dup_resp_stream
        .next()
        .await
        .expect("Should receive response")?;
    match first_resp.response {
        Some(PublishResponse::EndStream(end)) => {
            assert_eq!(end.status, EndCode::DuplicateBlock as i32);
        }
        other => panic!("Expected EndStream DuplicateBlock, got {:?}", other),
    }

    Ok(())
}

#[tokio::test]
#[serial]
async fn test_future_block_rejected() -> Result<()> {
    let ctx = TestContext::new().await?;
    let mut client = ctx.publisher_client().await?;

    let block_bytes = BlockBuilder::new(1).build();
    let block_proto: rock_node_protobufs::com::hedera::hapi::block::stream::Block =
        Message::decode(block_bytes.as_slice())?;

    let (tx, rx) = mpsc::channel(1);
    tx.send(PublishStreamRequest {
        request: Some(PublishRequest::BlockItems(BlockItemSet {
            block_items: vec![block_proto.items[0].clone()],
        })),
    })
    .await?;
    drop(tx);

    let mut response_stream = client
        .publish_block_stream(ReceiverStream::new(rx))
        .await?
        .into_inner();

    let response = response_stream
        .next()
        .await
        .expect("Should receive a response for a future block")?;

    match response.response {
        Some(PublishResponse::EndStream(end)) => {
            assert_eq!(
                end.status,
                EndCode::Behind as i32,
                "Expected 'Behind' error code"
            );
        }
        other => panic!("Expected EndStream Behind, got {:?}", other),
    }

    Ok(())
}
