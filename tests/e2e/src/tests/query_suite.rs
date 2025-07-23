use crate::common::{block_builder::BlockBuilder, TestContext};
use anyhow::Result;
use prost::Message;
use rock_node_protobufs::{
    org::hiero::block::api::{
        publish_stream_request::Request as PublishRequest, BlockItemSet, PublishStreamRequest,
    },
    proto::{
        account_id::Account, query::Query, AccountId, CryptoGetInfoQuery, Query as TopLevelQuery,
        ResponseCodeEnum,
    },
};
use serial_test::serial;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;

#[tokio::test]
#[serial]
async fn test_state_query_after_publish() -> Result<()> {
    let ctx = TestContext::new().await?;
    let mut publish_client = ctx.publisher_client().await?;
    let mut query_client = ctx.query_client().await?;

    // 1. Build a block with a state change for account 0.0.999
    let block_bytes = BlockBuilder::new(0)
        .with_account_state_change(999, "test-memo")
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

    // 3. Query for the account info
    let account_id = AccountId {
        shard_num: 0,
        realm_num: 0,
        account: Some(Account::AccountNum(999)),
    };
    let query = TopLevelQuery {
        query: Some(Query::CryptoGetInfo(CryptoGetInfoQuery {
            account_id: Some(account_id),
            header: None,
        })),
    };

    let response = query_client.get_account_info(query).await?.into_inner();

    // 4. Assert the response
    let crypto_response = match response.response {
        Some(rock_node_protobufs::proto::response::Response::CryptoGetInfo(info)) => info,
        other => panic!("Expected CryptoGetInfo response, got {:?}", other),
    };

    assert_eq!(
        crypto_response
            .header
            .as_ref()
            .unwrap()
            .node_transaction_precheck_code,
        ResponseCodeEnum::Ok as i32,
        "Response code should be OK"
    );

    let account_info = crypto_response
        .account_info
        .expect("Response should contain AccountInfo");
    assert_eq!(
        account_info.memo, "test-memo",
        "The account memo is incorrect"
    );
    assert_eq!(
        account_info.account_id.unwrap().account.unwrap(),
        Account::AccountNum(999)
    );

    Ok(())
}
