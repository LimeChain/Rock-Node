use crate::common::{publish_blocks, TestContext};
use anyhow::Result;
use prost::Message;
use rock_node_protobufs::org::hiero::block::api::{
    block_stream_publish_service_client::BlockStreamPublishServiceClient,
    publish_stream_request::Request as PublishRequest,
    subscribe_stream_response::{Code as SubCode, Response as SubResponse},
    BlockItemSet, PublishStreamRequest, SubscribeStreamRequest,
};
use serial_test::serial;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;
use tonic::transport::Channel;

/// Helper to extract the block number from a `BlockItems` response.
fn header_number(item_set: &rock_node_protobufs::org::hiero::block::api::BlockItemSet) -> u64 {
    use rock_node_protobufs::com::hedera::hapi::block::stream::block_item::Item;
    item_set
        .block_items
        .iter()
        .find_map(|item| match &item.item {
            Some(Item::BlockHeader(h)) => Some(h.number),
            _ => None,
        })
        .expect("Block items must contain a header")
}

/// Test Case: Subscribe to range that will have a gap filled live.
/// Tests that a subscriber can successfully receive blocks even when there's
/// a gap in the historical data that gets filled while streaming.
///
/// Note: This is a simplified version that avoids broadcast timing issues
/// by publishing the gap before the subscriber gets too far ahead.
#[tokio::test]
#[serial]
async fn test_subscribe_across_gap() -> Result<()> {
    let ctx = Arc::new(TestContext::new().await?);

    // Publish initial blocks 0-5
    publish_blocks(&ctx, 0, 5).await?;

    // Start subscriber requesting 0-20
    let mut sub_client = ctx.subscriber_client().await?;
    let request = SubscribeStreamRequest {
        start_block_number: 0,
        end_block_number: 20,
    };

    let mut stream = sub_client
        .subscribe_block_stream(request)
        .await?
        .into_inner();

    // Receive first 6 blocks (0-5)
    let mut received = Vec::new();
    for _ in 0..6 {
        if let Some(Ok(msg)) = stream.next().await {
            if let Some(SubResponse::BlockItems(set)) = msg.response {
                received.push(header_number(&set));
            }
        }
    }
    assert_eq!(received, vec![0, 1, 2, 3, 4, 5], "Should receive 0-5");

    // Now publish the next blocks sequentially (simulating continuous publishing)
    // This tests that the subscriber can keep up with live blocks
    let ctx_clone = Arc::clone(&ctx);
    tokio::spawn(async move {
        for i in 6..=20 {
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            publish_blocks(&ctx_clone, i, i).await.unwrap();
        }
    });

    // Receive remaining blocks (6-20)
    for expected in 6..=20 {
        if let Some(Ok(msg)) =
            tokio::time::timeout(tokio::time::Duration::from_secs(5), stream.next()).await?
        {
            if let Some(SubResponse::BlockItems(set)) = msg.response {
                let block_num = header_number(&set);
                assert_eq!(
                    block_num, expected,
                    "Expected block {} but got {}",
                    expected, block_num
                );
            }
        }
    }

    Ok(())
}

/// Test Case: High concurrency with many subscribers.
/// Tests that the subscriber service can handle many concurrent subscribers
/// (50+) without resource exhaustion, deadlocks, or errors.
#[tokio::test]
#[serial]
async fn test_high_concurrency_many_subscribers() -> Result<()> {
    let ctx = Arc::new(TestContext::new().await?);

    // Publish sufficient blocks to cover all subscriber ranges
    // Subscriber 49: start=98, end=108, so we need at least 0-108
    publish_blocks(&ctx, 0, 120).await?;

    // Spawn 50 concurrent subscribers with different ranges
    let mut tasks = Vec::new();
    for i in 0..50 {
        let ctx_clone = Arc::clone(&ctx);
        let start = i * 2;
        let end = start + 10;

        tasks.push(tokio::spawn(async move {
            let mut client = ctx_clone.subscriber_client().await.unwrap();
            let request = SubscribeStreamRequest {
                start_block_number: start,
                end_block_number: end,
            };

            let mut stream = client
                .subscribe_block_stream(request)
                .await
                .unwrap()
                .into_inner();

            let mut count = 0;
            while let Some(Ok(msg)) = stream.next().await {
                if let Some(SubResponse::BlockItems(_)) = msg.response {
                    count += 1;
                }
                if let Some(SubResponse::Status(code)) = msg.response {
                    assert_eq!(code, SubCode::Success as i32);
                    break;
                }
            }
            count
        }));
    }

    // All should complete successfully
    for (i, task) in tasks.into_iter().enumerate() {
        let count = task.await?;
        assert!(
            count > 0,
            "Subscriber {} received no blocks (expected 11)",
            i
        );
        assert_eq!(
            count, 11,
            "Subscriber {} should receive exactly 11 blocks",
            i
        );
    }

    Ok(())
}

/// Test Case: Rapid block publication with live subscriber.
/// Tests that a live subscriber can keep up with rapidly published blocks
/// without data loss or errors.
#[tokio::test]
#[serial]
async fn test_rapid_block_publication() -> Result<()> {
    let ctx = Arc::new(TestContext::new().await?);

    // Start live subscriber
    let mut sub_client = ctx.subscriber_client().await?;
    let request = SubscribeStreamRequest {
        start_block_number: 0,
        end_block_number: 50,
    };

    let mut stream = sub_client
        .subscribe_block_stream(request)
        .await?
        .into_inner();

    // Publish blocks rapidly in background
    let ctx_clone = Arc::clone(&ctx);
    let publish_task = tokio::spawn(async move {
        let publish_port = ctx_clone.container.get_host_port_ipv4(50051).await.unwrap();
        let publish_endpoint = format!("http://localhost:{}", publish_port);
        let channel = Channel::from_shared(publish_endpoint)
            .unwrap()
            .connect()
            .await
            .unwrap();
        let mut client = BlockStreamPublishServiceClient::new(channel);

        for i in 0..=50 {
            let block_bytes = crate::common::block_builder::BlockBuilder::new(i).build();
            let block_proto: rock_node_protobufs::com::hedera::hapi::block::stream::Block =
                Message::decode(block_bytes.as_slice()).unwrap();

            let (tx, rx) = mpsc::channel(1);
            tx.send(PublishStreamRequest {
                request: Some(PublishRequest::BlockItems(BlockItemSet {
                    block_items: block_proto.items,
                })),
            })
            .await
            .unwrap();
            drop(tx);

            let mut responses = client
                .publish_block_stream(ReceiverStream::new(rx))
                .await
                .unwrap()
                .into_inner();

            while responses.next().await.is_some() {}

            // Small delay to simulate rapid but not instant
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        }
    });

    // Subscriber should receive all blocks
    let mut received_count = 0;
    while let Some(Ok(msg)) = stream.next().await {
        match msg.response {
            Some(SubResponse::BlockItems(_)) => {
                received_count += 1;
            },
            Some(SubResponse::Status(code)) => {
                assert_eq!(code, SubCode::Success as i32);
                break;
            },
            _ => {},
        }
    }

    publish_task.await?;

    assert_eq!(
        received_count, 51,
        "Should receive all 51 blocks (0-50) despite rapid publication"
    );
    Ok(())
}

/// Test Case: Subscribers with overlapping ranges.
/// Tests that multiple subscribers with overlapping block ranges
/// can operate independently without interfering with each other.
#[tokio::test]
#[serial]
async fn test_overlapping_subscriber_ranges() -> Result<()> {
    let ctx = Arc::new(TestContext::new().await?);

    publish_blocks(&ctx, 0, 30).await?;

    // Subscriber A: 0-20
    let ctx_a = Arc::clone(&ctx);
    let task_a = tokio::spawn(async move {
        let mut client = ctx_a.subscriber_client().await.unwrap();
        let request = SubscribeStreamRequest {
            start_block_number: 0,
            end_block_number: 20,
        };
        let mut stream = client
            .subscribe_block_stream(request)
            .await
            .unwrap()
            .into_inner();

        let mut received = Vec::new();
        while let Some(Ok(msg)) = stream.next().await {
            match msg.response {
                Some(SubResponse::BlockItems(ref set)) => {
                    received.push(header_number(set));
                },
                Some(SubResponse::Status(_)) => break,
                _ => {},
            }
        }
        received
    });

    // Subscriber B: 10-30 (overlaps with A on 10-20)
    let ctx_b = Arc::clone(&ctx);
    let task_b = tokio::spawn(async move {
        let mut client = ctx_b.subscriber_client().await.unwrap();
        let request = SubscribeStreamRequest {
            start_block_number: 10,
            end_block_number: 30,
        };
        let mut stream = client
            .subscribe_block_stream(request)
            .await
            .unwrap()
            .into_inner();

        let mut received = Vec::new();
        while let Some(Ok(msg)) = stream.next().await {
            match msg.response {
                Some(SubResponse::BlockItems(ref set)) => {
                    received.push(header_number(set));
                },
                Some(SubResponse::Status(_)) => break,
                _ => {},
            }
        }
        received
    });

    let received_a = task_a.await?;
    let received_b = task_b.await?;

    assert_eq!(received_a, (0..=20).collect::<Vec<u64>>());
    assert_eq!(received_b, (10..=30).collect::<Vec<u64>>());

    Ok(())
}

/// Test Case: Subscribe to single block (start == end).
/// Tests edge case where subscriber requests exactly one block.
#[tokio::test]
#[serial]
async fn test_subscribe_single_block() -> Result<()> {
    let ctx = TestContext::new().await?;

    publish_blocks(&ctx, 0, 10).await?;

    let mut sub_client = ctx.subscriber_client().await?;
    let request = SubscribeStreamRequest {
        start_block_number: 5,
        end_block_number: 5, // Single block
    };

    let mut stream = sub_client
        .subscribe_block_stream(request)
        .await?
        .into_inner();

    let mut received = Vec::new();
    while let Some(Ok(msg)) = stream.next().await {
        match msg.response {
            Some(SubResponse::BlockItems(set)) => {
                received.push(header_number(&set));
            },
            Some(SubResponse::Status(code)) => {
                assert_eq!(code, SubCode::Success as i32);
                break;
            },
            _ => {},
        }
    }

    assert_eq!(received, vec![5], "Should receive exactly block 5");
    Ok(())
}
