use crate::common::{block_builder::BlockBuilder, TestContext};
use anyhow::Result;
use prost::Message;
use rock_node_protobufs::{
    org::hiero::block::api::{
        publish_stream_request::Request as PublishRequest, BlockItemSet, PublishStreamRequest,
    },
    proto::{
        query::Query, response, Query as TopLevelQuery, ResponseCodeEnum, ScheduleGetInfoQuery,
        ScheduleId,
    },
};
use serial_test::serial;
use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, StreamExt};

/// Test Case: Get Schedule Info
/// Objective: Verify that the `getScheduleInfo` query returns the correct schedule info.
#[tokio::test]
#[serial]
async fn test_get_schedule_info_successfully() -> Result<()> {
    let ctx = TestContext::new().await?;
    let mut publish_client = ctx.publisher_client().await?;
    let mut query_client = ctx.schedule_client().await?;

    // 1. Build a block with a state change for schedule 0.0.456
    let block_bytes = BlockBuilder::new(0)
        .with_schedule_state_change(456, "test-schedule-memo")
        .build();
    let block_proto: rock_node_protobufs::com::hedera::hapi::block::stream::Block =
        Message::decode(block_bytes.as_slice())?;

    // 2. Publish the block
    let (tx, rx) = mpsc::channel(1);
    tx.send(PublishStreamRequest {
        request: Some(PublishRequest::BlockItems(BlockItemSet {
            block_items: block_proto.items,
        })),
    })
    .await?;
    drop(tx);

    let mut responses = publish_client
        .publish_block_stream(ReceiverStream::new(rx))
        .await?
        .into_inner();
    while responses.next().await.is_some() {} // Drain acks

    // 3. Query for the schedule info
    let schedule_id = ScheduleId {
        shard_num: 0,
        realm_num: 0,
        schedule_num: 456,
    };
    let query = TopLevelQuery {
        query: Some(Query::ScheduleGetInfo(ScheduleGetInfoQuery {
            schedule_id: Some(schedule_id),
            header: None,
        })),
    };

    let response = query_client.get_schedule_info(query).await?.into_inner();

    // 4. Assert the response
    let schedule_response = match response.response {
        Some(response::Response::ScheduleGetInfo(resp)) => resp,
        other => panic!("Expected ScheduleGetInfo response, got {:?}", other),
    };

    assert_eq!(
        schedule_response
            .header
            .as_ref()
            .unwrap()
            .node_transaction_precheck_code,
        ResponseCodeEnum::Ok as i32
    );

    let schedule_info = schedule_response.schedule_info.unwrap();
    assert_eq!(schedule_info.schedule_id.unwrap(), schedule_id);
    assert_eq!(schedule_info.memo, "test-schedule-memo");

    Ok(())
}
