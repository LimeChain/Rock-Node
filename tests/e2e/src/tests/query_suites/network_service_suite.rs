use crate::common::{block_builder::BlockBuilder, TestContext};
use anyhow::Result;
use prost::Message;
use rock_node_protobufs::{
    org::hiero::block::api::{
        publish_stream_request::Request as PublishRequest, BlockItemSet, PublishStreamRequest,
    },
    proto::{
        query::Query, response, AccountId, GetAccountDetailsQuery, NetworkGetVersionInfoQuery,
        Query as TopLevelQuery, ResponseCodeEnum,
    },
};
use serial_test::serial;
use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, StreamExt};

/// Test Case: Get Version Info
/// Objective: Verify that the `getVersionInfo` query returns the correct version info.
#[tokio::test]
#[serial]
async fn test_get_version_info_successfully() -> Result<()> {
    let ctx = TestContext::new().await?;
    let mut publish_client = ctx.publisher_client().await?;
    let mut query_client = ctx.network_client().await?;

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

    let mut responses = publish_client
        .publish_block_stream(ReceiverStream::new(rx))
        .await?
        .into_inner();
    while responses.next().await.is_some() {}

    let query = TopLevelQuery {
        query: Some(Query::NetworkGetVersionInfo(NetworkGetVersionInfoQuery {
            header: None,
        })),
    };

    let response = query_client.get_version_info(query).await?.into_inner();

    let version_response = match response.response {
        Some(response::Response::NetworkGetVersionInfo(resp)) => resp,
        other => panic!("Expected NetworkGetVersionInfo response, got {:?}", other),
    };
    println!("version_response: {:?}", version_response);
    assert_eq!(
        version_response
            .header
            .as_ref()
            .unwrap()
            .node_transaction_precheck_code,
        ResponseCodeEnum::Ok as i32
    );

    assert!(version_response.hapi_proto_version.is_some());
    assert!(version_response.hedera_services_version.is_some());

    Ok(())
}

/// Test Case: Get Account Details
/// Objective: Verify that the `getAccountDetails` query returns the correct account details.
#[tokio::test]
#[serial]
async fn test_get_account_details_successfully() -> Result<()> {
    let ctx = TestContext::new().await?;
    let mut publish_client = ctx.publisher_client().await?;
    let mut query_client = ctx.network_client().await?;

    let block_bytes = BlockBuilder::new(0)
        .with_account_state_change(888, "details-test")
        .build();
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

    let mut responses = publish_client
        .publish_block_stream(ReceiverStream::new(rx))
        .await?
        .into_inner();
    while responses.next().await.is_some() {}

    let account_id = AccountId {
        shard_num: 0,
        realm_num: 0,
        account: Some(rock_node_protobufs::proto::account_id::Account::AccountNum(
            888,
        )),
    };
    let query = TopLevelQuery {
        query: Some(Query::AccountDetails(GetAccountDetailsQuery {
            account_id: Some(account_id.clone()),
            header: None,
        })),
    };

    let response = query_client.get_account_details(query).await?.into_inner();

    let details_response = match response.response {
        Some(response::Response::AccountDetails(resp)) => resp,
        other => panic!("Expected GetAccountDetailsResponse, got {:?}", other),
    };

    assert_eq!(
        details_response
            .header
            .as_ref()
            .unwrap()
            .node_transaction_precheck_code,
        ResponseCodeEnum::Ok as i32
    );

    let account_details = details_response.account_details.unwrap();
    assert_eq!(account_details.account_id.unwrap(), account_id);
    assert_eq!(account_details.memo, "details-test");
    assert_eq!(account_details.balance, 888);

    Ok(())
}
