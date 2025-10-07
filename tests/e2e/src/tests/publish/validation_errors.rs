use crate::common::{block_builder::BlockBuilder, TestContext};
use anyhow::Result;
use prost::Message;
use rock_node_protobufs::org::hiero::block::api::{
    publish_stream_request::Request as PublishRequest,
    publish_stream_response::{end_of_stream::Code as EndCode, Response as PublishResponse},
    BlockItemSet, PublishStreamRequest,
};
use serial_test::serial;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, StreamExt};

/// Verifies that when a publisher tries to publish an already-persisted block,
/// they receive EndStream with DuplicateBlock status
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
        .expect("Should receive response for duplicate block")?;
    match first_resp.response {
        Some(PublishResponse::EndStream(end)) => {
            assert_eq!(end.status, EndCode::DuplicateBlock as i32);
            assert_eq!(
                end.block_number, 0,
                "EndStream should report last persisted block"
            );
        },
        other => panic!("Expected EndStream DuplicateBlock, got {:?}", other),
    }

    println!("✅ Duplicate block correctly rejected");
    Ok(())
}

/// Verifies that empty block item sets are rejected with validation error
#[tokio::test]
#[serial]
async fn test_empty_block_items_rejected() -> Result<()> {
    let ctx = TestContext::new().await?;
    let mut client = ctx.publisher_client().await?;

    let (tx, rx) = mpsc::channel(1);

    // Send empty block items
    tx.send(PublishStreamRequest {
        request: Some(PublishRequest::BlockItems(BlockItemSet {
            block_items: vec![],
        })),
    })
    .await?;
    drop(tx);

    let mut responses = client
        .publish_block_stream(ReceiverStream::new(rx))
        .await?
        .into_inner();

    // Should receive EndStream with error
    let response = tokio::time::timeout(Duration::from_secs(2), responses.next())
        .await?
        .ok_or(anyhow::anyhow!("No response received"))??;

    match response.response {
        Some(PublishResponse::EndStream(end)) => {
            assert_eq!(
                end.status,
                EndCode::Error as i32,
                "Expected Error status for empty block items"
            );
        },
        other => panic!("Expected EndStream with Error, got {:?}", other),
    }

    println!("✅ Empty block items correctly rejected");
    Ok(())
}

/// Category 2 - Test 2.2: Missing Header Validation
/// Verifies that block items without header are rejected
#[tokio::test]
#[serial]
async fn test_missing_header_rejected() -> Result<()> {
    let ctx = TestContext::new().await?;
    let mut client = ctx.publisher_client().await?;

    let (tx, rx) = mpsc::channel(1);

    // Create a block and send only the proof (no header)
    let block_bytes = BlockBuilder::new(0).build();
    let block_proto: rock_node_protobufs::com::hedera::hapi::block::stream::Block =
        Message::decode(block_bytes.as_slice())?;

    // Send only proof item (last item), skip header
    let proof_only = vec![block_proto.items.last().unwrap().clone()];

    tx.send(PublishStreamRequest {
        request: Some(PublishRequest::BlockItems(BlockItemSet {
            block_items: proof_only,
        })),
    })
    .await?;
    drop(tx);

    let mut responses = client
        .publish_block_stream(ReceiverStream::new(rx))
        .await?
        .into_inner();

    // Should receive EndStream with error
    let response = tokio::time::timeout(Duration::from_secs(2), responses.next())
        .await?
        .ok_or(anyhow::anyhow!("No response received"))??;

    match response.response {
        Some(PublishResponse::EndStream(end)) => {
            assert_eq!(
                end.status,
                EndCode::Error as i32,
                "Expected Error status for missing header"
            );
        },
        other => panic!("Expected EndStream with Error, got {:?}", other),
    }

    println!("✅ Missing header correctly rejected");
    Ok(())
}

/// Verifies that blocks behind the persisted chain are rejected
#[tokio::test]
#[serial]
async fn test_behind_block_rejected() -> Result<()> {
    let ctx = TestContext::new().await?;

    // First, publish block 0 to establish chain
    let mut client1 = ctx.publisher_client().await?;
    let (tx1, rx1) = mpsc::channel(1);

    let block0_bytes = BlockBuilder::new(0).build();
    let block0_proto: rock_node_protobufs::com::hedera::hapi::block::stream::Block =
        Message::decode(block0_bytes.as_slice())?;

    tx1.send(PublishStreamRequest {
        request: Some(PublishRequest::BlockItems(BlockItemSet {
            block_items: block0_proto.items,
        })),
    })
    .await?;
    drop(tx1);

    let mut responses1 = client1
        .publish_block_stream(ReceiverStream::new(rx1))
        .await?
        .into_inner();

    // Wait for ACK
    while let Some(resp) = responses1.next().await {
        let resp = resp?;
        if matches!(resp.response, Some(PublishResponse::Acknowledgement(_))) {
            break;
        }
    }

    // Now try to publish block 0 again from different publisher (behind)
    let mut client2 = ctx.publisher_client().await?;
    let (tx2, rx2) = mpsc::channel(1);

    let block0_again_bytes = BlockBuilder::new(0).build();
    let block0_again_proto: rock_node_protobufs::com::hedera::hapi::block::stream::Block =
        Message::decode(block0_again_bytes.as_slice())?;

    tx2.send(PublishStreamRequest {
        request: Some(PublishRequest::BlockItems(BlockItemSet {
            block_items: vec![block0_again_proto.items[0].clone()],
        })),
    })
    .await?;
    drop(tx2);

    let mut responses2 = client2
        .publish_block_stream(ReceiverStream::new(rx2))
        .await?
        .into_inner();

    // Should receive EndStream with DuplicateBlock
    let response = tokio::time::timeout(Duration::from_secs(2), responses2.next())
        .await?
        .ok_or(anyhow::anyhow!("No response received"))??;

    match response.response {
        Some(PublishResponse::EndStream(end)) => {
            assert_eq!(
                end.status,
                EndCode::DuplicateBlock as i32,
                "Expected DuplicateBlock status"
            );
        },
        other => panic!("Expected EndStream with DuplicateBlock, got {:?}", other),
    }

    println!("✅ Behind block correctly rejected");
    Ok(())
}

/// Verifies that oversized block item sets are rejected
///
/// NOTE: This test requires max_items_per_set to be explicitly set in config.e2e.toml
#[tokio::test]
#[serial]
async fn test_oversized_block_items_rejected() -> Result<()> {
    let ctx = TestContext::new().await?;
    let mut client = ctx.publisher_client().await?;

    let (tx, rx) = mpsc::channel(1);

    // Create a block with header
    let block_bytes = BlockBuilder::new(0).build();
    let block_proto: rock_node_protobufs::com::hedera::hapi::block::stream::Block =
        Message::decode(block_bytes.as_slice())?;

    // Create excessive number of items (more than max_items_per_set of 10000)
    let mut large_items = vec![block_proto.items[0].clone()]; // header

    // Add items to exceed the limit
    for _ in 1..10001 {
        large_items.push(block_proto.items.last().unwrap().clone());
    }

    tx.send(PublishStreamRequest {
        request: Some(PublishRequest::BlockItems(BlockItemSet {
            block_items: large_items,
        })),
    })
    .await?;
    drop(tx);

    let mut responses = client
        .publish_block_stream(ReceiverStream::new(rx))
        .await?
        .into_inner();

    // Should receive EndStream with error
    let response = tokio::time::timeout(Duration::from_secs(2), responses.next())
        .await?
        .ok_or(anyhow::anyhow!("No response received"))??;

    match response.response {
        Some(PublishResponse::EndStream(end)) => {
            assert_eq!(
                end.status,
                EndCode::Error as i32,
                "Expected Error status for oversized block"
            );
        },
        other => panic!("Expected EndStream with Error, got {:?}", other),
    }

    println!("✅ Oversized block items correctly rejected");
    Ok(())
}
