use crate::common::{publish_blocks, TestContext};
use anyhow::Result;
use prost::Message;
use rock_node_protobufs::org::hiero::block::api::{
    block_stream_publish_service_client::BlockStreamPublishServiceClient,
    publish_stream_request::Request as PublishRequest, BlockItemSet, PublishStreamRequest,
};
use rock_node_protobufs::org::hiero::block::api::{
    subscribe_stream_response::{Code as SubCode, Response as SubResponse},
    SubscribeStreamRequest,
};
use serial_test::serial;
use tokio::sync::mpsc;
use tokio::time::Duration;
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

/// Test Case: finite historical range.
#[tokio::test]
#[serial]
async fn test_historical_stream() -> Result<()> {
    let ctx = TestContext::new().await?;

    // Pre-populate blocks 0..=20.
    publish_blocks(&ctx, 0, 20).await?;

    let mut sub_client = ctx.subscriber_client().await?;
    let request = SubscribeStreamRequest {
        start_block_number: 5,
        end_block_number: 10,
    };

    let mut stream = sub_client
        .subscribe_block_stream(request)
        .await?
        .into_inner();
    let mut expected = 5u64;
    let mut received = 0u64;

    while let Some(msg) = stream.next().await {
        let msg = msg?;
        match msg.response {
            Some(SubResponse::BlockItems(set)) => {
                let num = header_number(&set);
                assert_eq!(num, expected, "Blocks received out of order");
                expected += 1;
                received += 1;
            }
            Some(SubResponse::Status(code)) => {
                assert_eq!(code, SubCode::ReadStreamSuccess as i32);
                break;
            }
            other => panic!("Unexpected response: {:?}", other),
        }
    }

    assert_eq!(received, 6, "Should have received 6 blocks (5..=10)");
    Ok(())
}

/// Test Case: live streaming after historical catch-up.
#[tokio::test]
#[serial]
async fn test_historical_to_live_stream() -> Result<()> {
    let ctx = TestContext::new().await?;

    // Publish first batch 0..=5 so they are in persistence before subscription.
    publish_blocks(&ctx, 0, 5).await?;

    let mut sub_client = ctx.subscriber_client().await?;
    let request = SubscribeStreamRequest {
        start_block_number: 0,
        end_block_number: 10,
    };
    let mut stream = sub_client
        .subscribe_block_stream(request)
        .await?
        .into_inner();

    // Determine the endpoint for the publish service and spawn a task that will
    // publish the remaining blocks after a short delay.
    let publish_port = ctx.container.get_host_port_ipv4(50051).await?;
    let publish_endpoint = format!("http://localhost:{}", publish_port);

    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let channel = Channel::from_shared(publish_endpoint)
            .unwrap()
            .connect()
            .await
            .unwrap();
        let mut client = BlockStreamPublishServiceClient::new(channel);
        for i in 6..=10 {
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
        }
    });

    let mut expected = 0u64;
    while let Some(msg) = stream.next().await {
        let msg = msg?;
        match msg.response {
            Some(SubResponse::BlockItems(set)) => {
                let num = header_number(&set);
                assert_eq!(num, expected);
                expected += 1;
            }
            Some(SubResponse::Status(code)) => {
                assert_eq!(code, SubCode::ReadStreamSuccess as i32);
                break;
            }
            other => panic!("Unexpected response: {:?}", other),
        }
    }

    assert_eq!(expected, 11, "Should have received 11 blocks (0..=10)");
    Ok(())
}

/// Test Case: start > end produces invalid end block error.
#[tokio::test]
#[serial]
async fn test_invalid_end_block_number() -> Result<()> {
    let ctx = TestContext::new().await?;

    let mut sub_client = ctx.subscriber_client().await?;
    let request = SubscribeStreamRequest {
        start_block_number: 10,
        end_block_number: 5,
    };

    let mut stream = sub_client
        .subscribe_block_stream(request)
        .await?
        .into_inner();
    let first = stream.next().await.expect("Expected a response")?;
    match first.response {
        Some(SubResponse::Status(code)) => {
            assert_eq!(code, SubCode::ReadStreamInvalidEndBlockNumber as i32);
        }
        other => panic!("Unexpected response: {:?}", other),
    }
    assert!(
        stream.next().await.is_none(),
        "Stream should have terminated"
    );
    Ok(())
}
