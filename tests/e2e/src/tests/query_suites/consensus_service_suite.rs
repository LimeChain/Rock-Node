use crate::common::{block_builder::BlockBuilder, TestContext};
use anyhow::Result;
use prost::Message;
use rock_node_protobufs::{
    org::hiero::block::api::{
        publish_stream_request::Request as PublishRequest, BlockItemSet, PublishStreamRequest,
    },
    proto::{
        query::Query, response, ConsensusGetTopicInfoQuery, Query as TopLevelQuery,
        ResponseCodeEnum, TopicId,
    },
};
use serial_test::serial;
use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, StreamExt};

/// Test Case: Get Topic Info
/// Objective: Verify that the `getTopicInfo` query returns the correct topic info
/// for a topic after its state has been updated.
#[tokio::test]
#[serial]
async fn test_get_topic_info_successfully() -> Result<()> {
    let ctx = TestContext::new().await?;
    let mut publish_client = ctx.publisher_client().await?;
    let mut query_client = ctx.consensus_client().await?;

    // 1. Build a block with a state change for topic 0.0.123
    let block_bytes = BlockBuilder::new(0)
        .with_topic_state_change(123, "test-topic-memo")
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
    // Drain responses to ensure persistence
    while responses.next().await.is_some() {}

    // 3. Query for the topic info
    let topic_id = TopicId {
        shard_num: 0,
        realm_num: 0,
        topic_num: 123,
    };
    let query = TopLevelQuery {
        query: Some(Query::ConsensusGetTopicInfo(ConsensusGetTopicInfoQuery {
            topic_id: Some(topic_id),
            header: None,
        })),
    };

    let response = query_client.get_topic_info(query).await?.into_inner();

    // 4. Assert the response
    let consensus_response = match response.response {
        Some(response::Response::ConsensusGetTopicInfo(info)) => info,
        other => panic!("Expected ConsensusGetTopicInfo response, got {:?}", other),
    };

    assert_eq!(
        consensus_response
            .header
            .as_ref()
            .unwrap()
            .node_transaction_precheck_code,
        ResponseCodeEnum::Ok as i32,
        "Response code should be OK"
    );

    let topic_info = consensus_response
        .topic_info
        .expect("Response should contain TopicInfo");
    assert_eq!(
        topic_info.memo, "test-topic-memo",
        "The topic memo is incorrect"
    );
    assert_eq!(consensus_response.topic_id.unwrap().topic_num, 123);

    Ok(())
}
