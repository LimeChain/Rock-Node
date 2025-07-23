use crate::common::{block_builder::BlockBuilder, TestContext};
use anyhow::Result;
use prost::Message;
use rock_node_protobufs::org::hiero::block::api::{
    publish_stream_request::Request as PublishRequest,
    publish_stream_response::{
        end_of_stream::Code as EndCode, BlockAcknowledgement, Response as PublishResponse,
        SkipBlock,
    },
    BlockItemSet, PublishStreamRequest,
};
use serial_test::serial;
use std::sync::Arc;
use std::time::Duration;
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

/// Test Case: Duplicate block – sending an already persisted block returns EndStream DuplicateBlock.
#[tokio::test]
#[serial]
async fn test_duplicate_block_rejected() -> Result<()> {
    let ctx = TestContext::new().await?;
    let mut client = ctx.publisher_client().await?;

    // First, publish block 0 successfully
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
    // Drain the responses to ensure the block is processed and persisted.
    while let Some(resp) = responses.next().await {
        let resp = resp?;
        if matches!(resp.response, Some(PublishResponse::Acknowledgement(_))) {
            break;
        }
    }

    // Now, a new client tries to publish the same block's header
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
        .expect("Should receive response for duplicate block")?;
    match first_resp.response {
        Some(PublishResponse::EndStream(end)) => {
            assert_eq!(end.status, EndCode::DuplicateBlock as i32);
            assert_eq!(
                end.block_number, 0,
                "EndStream should report last persisted block"
            );
        }
        other => panic!("Expected EndStream DuplicateBlock, got {:?}", other),
    }

    Ok(())
}

/// Test Case: Race condition – two publishers send the same header, one wins, one gets SkipBlock.
#[tokio::test]
#[serial]
async fn test_multi_publisher_race_sends_skip_block() -> Result<()> {
    let ctx = TestContext::new().await?;

    // Create two clients
    let mut client_a = ctx.publisher_client().await?;
    let mut client_b = ctx.publisher_client().await?;

    // Prepare the request for block 0's header
    let header_request = {
        let block_bytes = BlockBuilder::new(0).build();
        let block_proto: rock_node_protobufs::com::hedera::hapi::block::stream::Block =
            Message::decode(block_bytes.as_slice())?;
        PublishStreamRequest {
            request: Some(PublishRequest::BlockItems(BlockItemSet {
                block_items: vec![block_proto.items[0].clone()],
            })),
        }
    };

    // Client A sends the header first
    let (tx_a, rx_a) = mpsc::channel(4);
    tx_a.send(header_request.clone()).await?;
    let mut responses_a = client_a
        .publish_block_stream(ReceiverStream::new(rx_a))
        .await?
        .into_inner();

    // Give a moment for the server to process A's header
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Client B sends the same header
    let (tx_b, rx_b) = mpsc::channel(4);
    tx_b.send(header_request.clone()).await?;
    let mut responses_b = client_b
        .publish_block_stream(ReceiverStream::new(rx_b))
        .await?
        .into_inner();

    // Client B (the loser) should immediately get a SkipBlock response
    let b_response = tokio::time::timeout(Duration::from_secs(1), responses_b.next())
        .await?
        .unwrap();
    assert!(
        matches!(
            b_response.unwrap().response,
            Some(PublishResponse::SkipBlock(SkipBlock { block_number: 0 }))
        ),
        "Client B should have received SkipBlock"
    );

    // Client A (the winner) should not have received any message yet, as it's waiting for the full block
    let a_result = tokio::time::timeout(Duration::from_millis(200), responses_a.next()).await;
    assert!(
        a_result.is_err(),
        "Client A should not have received a response yet"
    );

    Ok(())
}

/// Test Case: Multi-publisher happy path with broadcasted acknowledgements.
#[serial]
async fn test_multi_publisher_broadcast_ack() -> Result<()> {
    let ctx = Arc::new(TestContext::new().await?);

    let client_a_task = {
        let ctx = Arc::clone(&ctx);
        tokio::spawn(async move {
            let mut client = ctx.publisher_client().await.unwrap();
            let (tx, rx) = mpsc::channel(4);
            let mut responses = client
                .publish_block_stream(ReceiverStream::new(rx))
                .await
                .unwrap()
                .into_inner();

            // --- Block 0 ---
            // A sends block 0 and becomes primary
            let block0_bytes = BlockBuilder::new(0).build();
            let block0_proto: rock_node_protobufs::com::hedera::hapi::block::stream::Block =
                Message::decode(block0_bytes.as_slice()).unwrap();
            tx.send(PublishStreamRequest {
                request: Some(PublishRequest::BlockItems(BlockItemSet {
                    block_items: block0_proto.items,
                })),
            })
            .await
            .unwrap();

            // A should receive the ACK for block 0
            let resp0 = responses.next().await.unwrap().unwrap();
            assert!(matches!(
                resp0.response,
                Some(PublishResponse::Acknowledgement(BlockAcknowledgement {
                    block_number: 0,
                    ..
                }))
            ));

            // --- Block 1 ---
            // A sends header for block 1 but will be told to skip
            let block1_header_bytes = BlockBuilder::new(1).items();
            tx.send(PublishStreamRequest {
                request: Some(PublishRequest::BlockItems(BlockItemSet {
                    block_items: block1_header_bytes,
                })),
            })
            .await
            .unwrap();

            // A should receive SkipBlock for block 1, then an ACK for block 1 (sent by B)
            let resp1_skip = responses.next().await.unwrap().unwrap();
            assert!(matches!(
                resp1_skip.response,
                Some(PublishResponse::SkipBlock(SkipBlock { block_number: 1 }))
            ));

            let resp1_ack = responses.next().await.unwrap().unwrap();
            assert!(matches!(
                resp1_ack.response,
                Some(PublishResponse::Acknowledgement(BlockAcknowledgement {
                    block_number: 1,
                    ..
                }))
            ));
        })
    };

    let client_b_task = {
        let ctx = Arc::clone(&ctx);
        tokio::spawn(async move {
            let mut client = ctx.publisher_client().await.unwrap();
            let (tx, rx) = mpsc::channel(4);
            let mut responses = client
                .publish_block_stream(ReceiverStream::new(rx))
                .await
                .unwrap()
                .into_inner();

            // Give A time to become primary for block 0
            tokio::time::sleep(Duration::from_millis(100)).await;

            // --- Block 0 ---
            // B sends header for block 0 and is told to skip
            let block0_header_bytes = BlockBuilder::new(0).items();
            tx.send(PublishStreamRequest {
                request: Some(PublishRequest::BlockItems(BlockItemSet {
                    block_items: block0_header_bytes,
                })),
            })
            .await
            .unwrap();

            // B should receive SkipBlock, then an ACK for block 0 (sent by A)
            let resp0_skip = responses.next().await.unwrap().unwrap();
            assert!(matches!(
                resp0_skip.response,
                Some(PublishResponse::SkipBlock(SkipBlock { block_number: 0 }))
            ));

            let resp0_ack = responses.next().await.unwrap().unwrap();
            assert!(matches!(
                resp0_ack.response,
                Some(PublishResponse::Acknowledgement(BlockAcknowledgement {
                    block_number: 0,
                    ..
                }))
            ));

            // --- Block 1 ---
            // B sends block 1 and becomes primary
            let block1_bytes = BlockBuilder::new(1).build();
            let block1_proto: rock_node_protobufs::com::hedera::hapi::block::stream::Block =
                Message::decode(block1_bytes.as_slice()).unwrap();
            tx.send(PublishStreamRequest {
                request: Some(PublishRequest::BlockItems(BlockItemSet {
                    block_items: block1_proto.items,
                })),
            })
            .await
            .unwrap();

            // B should receive the ACK for block 1
            let resp1 = responses.next().await.unwrap().unwrap();
            assert!(matches!(
                resp1.response,
                Some(PublishResponse::Acknowledgement(BlockAcknowledgement {
                    block_number: 1,
                    ..
                }))
            ));
        })
    };

    client_a_task.await?;
    client_b_task.await?;

    Ok(())
}
