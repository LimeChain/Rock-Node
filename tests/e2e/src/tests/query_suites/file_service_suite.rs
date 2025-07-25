use crate::common::{block_builder::BlockBuilder, TestContext};
use anyhow::Result;
use prost::Message;
use rock_node_protobufs::{
    org::hiero::block::api::{
        publish_stream_request::Request as PublishRequest, BlockItemSet, PublishStreamRequest,
    },
    proto::{
        query::Query, response, FileGetContentsQuery, FileGetInfoQuery, FileId,
        Query as TopLevelQuery, ResponseCodeEnum,
    },
};
use serial_test::serial;
use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, StreamExt};

/// Test Case: Get File Info and Content
/// Objective: Verify that the `getFileInfo` and `getFileContent` queries return
/// correct data after a file state has been updated.
#[tokio::test]
#[serial]
async fn test_get_file_info_and_content_successfully() -> Result<()> {
    let ctx = TestContext::new().await?;
    let mut publish_client = ctx.publisher_client().await?;
    let mut query_client = ctx.file_client().await?;

    let file_content = "This is the content of the file.".as_bytes();

    // 1. Build a block with a state change for file 0.0.777
    let block_bytes = BlockBuilder::new(0)
        .with_file_state_change(777, "test-file-memo", file_content)
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

    // 3. Query for the file info
    let file_id = FileId {
        shard_num: 0,
        realm_num: 0,
        file_num: 777,
    };
    let info_query = TopLevelQuery {
        query: Some(Query::FileGetInfo(FileGetInfoQuery {
            file_id: Some(file_id.clone()),
            header: None,
        })),
    };
    let info_response = query_client.get_file_info(info_query).await?.into_inner();

    // 4. Assert the info response
    let file_info_resp = match info_response.response {
        Some(response::Response::FileGetInfo(resp)) => resp,
        other => panic!("Expected FileGetInfo response, got {:?}", other),
    };
    assert_eq!(
        file_info_resp
            .header
            .as_ref()
            .unwrap()
            .node_transaction_precheck_code,
        ResponseCodeEnum::Ok as i32
    );
    let file_info = file_info_resp.file_info.unwrap();
    assert_eq!(file_info.memo, "test-file-memo");
    assert_eq!(file_info.size, file_content.len() as i64);

    // 5. Query for the file content
    let content_query = TopLevelQuery {
        query: Some(Query::FileGetContents(FileGetContentsQuery {
            file_id: Some(file_id),
            header: None,
        })),
    };
    let content_response = query_client
        .get_file_content(content_query)
        .await?
        .into_inner();

    // 6. Assert the content response
    let file_content_resp = match content_response.response {
        Some(response::Response::FileGetContents(resp)) => resp,
        other => panic!("Expected FileGetContents response, got {:?}", other),
    };
    assert_eq!(
        file_content_resp
            .header
            .as_ref()
            .unwrap()
            .node_transaction_precheck_code,
        ResponseCodeEnum::Ok as i32
    );
    let contents = file_content_resp.file_contents.unwrap();
    assert_eq!(contents.contents, file_content);

    Ok(())
}
