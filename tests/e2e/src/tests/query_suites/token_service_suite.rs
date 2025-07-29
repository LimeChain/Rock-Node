use crate::common::{block_builder::BlockBuilder, TestContext};
use anyhow::Result;
use prost::Message;
use rock_node_protobufs::{
    org::hiero::block::api::{
        publish_stream_request::Request as PublishRequest, BlockItemSet, PublishStreamRequest,
    },
    proto::{
        query::Query, response, NftId, Query as TopLevelQuery, ResponseCodeEnum, TokenGetInfoQuery,
        TokenGetNftInfoQuery, TokenId,
    },
};
use serial_test::serial;
use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, StreamExt};

/// Test Case: Get Token Info (Happy Path)
/// Objective: Verify that the `getTokenInfo` query returns the correct information
/// for a token that exists in the state.
#[tokio::test]
#[serial]
async fn test_get_token_info_successfully() -> Result<()> {
    let ctx = TestContext::new().await?;
    let mut publish_client = ctx.publisher_client().await?;
    let mut query_client = ctx.token_client().await?;

    let block_bytes = BlockBuilder::new(0)
        .with_token_state_change(5001, "TST", "Test Token")
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
    while responses.next().await.is_some() {} // Drain acks

    let token_id = TokenId {
        shard_num: 0,
        realm_num: 0,
        token_num: 5001,
    };
    let query = TopLevelQuery {
        query: Some(Query::TokenGetInfo(TokenGetInfoQuery {
            header: None,
            token: Some(token_id.clone()),
        })),
    };

    let response = query_client.get_token_info(query).await?.into_inner();

    let token_response = match response.response {
        Some(response::Response::TokenGetInfo(resp)) => resp,
        other => panic!("Expected TokenGetInfo response, got {:?}", other),
    };

    assert_eq!(
        token_response
            .header
            .as_ref()
            .unwrap()
            .node_transaction_precheck_code,
        ResponseCodeEnum::Ok as i32
    );

    let token_info = token_response.token_info.unwrap();
    assert_eq!(token_info.token_id.unwrap(), token_id);
    assert_eq!(token_info.symbol, "TST");
    assert_eq!(token_info.name, "Test Token");

    Ok(())
}

/// Test Case: Get Token Info (Not Found)
/// Objective: Verify that `getTokenInfo` returns INVALID_TOKEN_ID for a non-existent token.
#[tokio::test]
#[serial]
async fn test_get_token_info_not_found() -> Result<()> {
    let ctx = TestContext::new().await?;
    let mut query_client = ctx.token_client().await?;

    let token_id = TokenId {
        shard_num: 0,
        realm_num: 0,
        token_num: 9999,
    };
    let query = TopLevelQuery {
        query: Some(Query::TokenGetInfo(TokenGetInfoQuery {
            header: None,
            token: Some(token_id),
        })),
    };

    let response = query_client.get_token_info(query).await?.into_inner();

    let token_response = match response.response {
        Some(response::Response::TokenGetInfo(resp)) => resp,
        other => panic!("Expected TokenGetInfo response, got {:?}", other),
    };

    assert_eq!(
        token_response
            .header
            .as_ref()
            .unwrap()
            .node_transaction_precheck_code,
        ResponseCodeEnum::InvalidTokenId as i32
    );
    assert!(token_response.token_info.is_none());

    Ok(())
}

/// Test Case: Get Token NFT Info (Happy Path)
/// Objective: Verify that `getTokenNftInfo` returns correct info for an existing NFT.
#[tokio::test]
#[serial]
async fn test_get_token_nft_info_successfully() -> Result<()> {
    let ctx = TestContext::new().await?;
    let mut publish_client = ctx.publisher_client().await?;
    let mut query_client = ctx.token_client().await?;

    let block_bytes = BlockBuilder::new(0)
        .with_nft_state_change(6001, 1, 101)
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
    while responses.next().await.is_some() {} // Drain acks

    let nft_id = NftId {
        token_id: Some(TokenId {
            shard_num: 0,
            realm_num: 0,
            token_num: 6001,
        }),
        serial_number: 1,
    };
    let query = TopLevelQuery {
        query: Some(Query::TokenGetNftInfo(TokenGetNftInfoQuery {
            header: None,
            nft_id: Some(nft_id.clone()),
        })),
    };

    let response = query_client.get_token_nft_info(query).await?.into_inner();
    let nft_response = match response.response {
        Some(response::Response::TokenGetNftInfo(resp)) => resp,
        other => panic!("Expected TokenGetNftInfo response, got {:?}", other),
    };

    assert_eq!(
        nft_response
            .header
            .as_ref()
            .unwrap()
            .node_transaction_precheck_code,
        ResponseCodeEnum::Ok as i32
    );

    let nft_info = nft_response.nft.unwrap();
    assert_eq!(nft_info.nft_id.unwrap(), nft_id);
    assert_eq!(
        nft_info.account_id.unwrap().account.unwrap(),
        rock_node_protobufs::proto::account_id::Account::AccountNum(101)
    );
    Ok(())
}

/// Test Case: Get Token NFT Info (Not Found)
/// Objective: Verify `getTokenNftInfo` returns INVALID_NFT_ID for a non-existent NFT.
#[tokio::test]
#[serial]
async fn test_get_token_nft_info_not_found() -> Result<()> {
    let ctx = TestContext::new().await?;
    let mut query_client = ctx.token_client().await?;

    let nft_id = NftId {
        token_id: Some(TokenId {
            token_num: 9999,
            ..Default::default()
        }),
        serial_number: 1,
    };
    let query = TopLevelQuery {
        query: Some(Query::TokenGetNftInfo(TokenGetNftInfoQuery {
            header: None,
            nft_id: Some(nft_id),
        })),
    };

    let response = query_client.get_token_nft_info(query).await?.into_inner();

    let nft_response = match response.response {
        Some(response::Response::TokenGetNftInfo(resp)) => resp,
        other => panic!("Expected TokenGetNftInfo response, got {:?}", other),
    };

    assert_eq!(
        nft_response
            .header
            .as_ref()
            .unwrap()
            .node_transaction_precheck_code,
        ResponseCodeEnum::InvalidNftId as i32
    );
    assert!(nft_response.nft.is_none());

    Ok(())
}
