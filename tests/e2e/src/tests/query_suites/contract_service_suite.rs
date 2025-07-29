use crate::common::{block_builder::BlockBuilder, TestContext};
use anyhow::Result;
use prost::Message;
use rock_node_protobufs::{
    org::hiero::block::api::{
        publish_stream_request::Request as PublishRequest, BlockItemSet, PublishStreamRequest,
    },
    proto::{
        contract_id, query::Query, response, ContractGetBytecodeQuery, ContractGetInfoQuery,
        ContractId, Query as TopLevelQuery, ResponseCodeEnum,
    },
};
use serial_test::serial;
use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, StreamExt};

/// Test Case: Get Contract Info (Happy Path)
/// Objective: Verify that `getContractInfo` returns correct data for an existing contract.
#[tokio::test]
#[serial]
async fn test_get_contract_info_successfully() -> Result<()> {
    let ctx = TestContext::new().await?;
    let mut publish_client = ctx.publisher_client().await?;
    let mut query_client = ctx.contract_client().await?;

    let block_bytes = BlockBuilder::new(0)
        .with_contract_state_change(8001, "test-contract-memo")
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

    let contract_id = ContractId {
        contract: Some(contract_id::Contract::ContractNum(8001)),
        ..Default::default()
    };
    let query = TopLevelQuery {
        query: Some(Query::ContractGetInfo(ContractGetInfoQuery {
            header: None,
            contract_id: Some(contract_id.clone()),
        })),
    };

    let response = query_client.get_contract_info(query).await?.into_inner();
    let contract_response = match response.response {
        Some(response::Response::ContractGetInfo(resp)) => resp,
        other => panic!("Expected ContractGetInfo response, got {:?}", other),
    };

    assert_eq!(
        contract_response
            .header
            .as_ref()
            .unwrap()
            .node_transaction_precheck_code,
        ResponseCodeEnum::Ok as i32
    );

    let contract_info = contract_response.contract_info.unwrap();
    assert_eq!(contract_info.contract_id.unwrap(), contract_id);
    assert_eq!(contract_info.memo, "test-contract-memo");
    Ok(())
}

/// Test Case: Get Contract Bytecode (Happy Path)
/// Objective: Verify `contractGetBytecode` returns correct bytecode for an existing contract.
#[tokio::test]
#[serial]
async fn test_get_contract_bytecode_successfully() -> Result<()> {
    let ctx = TestContext::new().await?;
    let mut publish_client = ctx.publisher_client().await?;
    let mut query_client = ctx.contract_client().await?;

    let bytecode = vec![0xFE, 0xED, 0xFA, 0xCE];
    let block_bytes = BlockBuilder::new(0)
        .with_bytecode_state_change(8002, &bytecode)
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

    let contract_id = ContractId {
        contract: Some(contract_id::Contract::ContractNum(8002)),
        ..Default::default()
    };
    let query = TopLevelQuery {
        query: Some(Query::ContractGetBytecode(ContractGetBytecodeQuery {
            header: None,
            contract_id: Some(contract_id),
        })),
    };

    let response = query_client
        .contract_get_bytecode(query)
        .await?
        .into_inner();
    let bytecode_response = match response.response {
        Some(response::Response::ContractGetBytecodeResponse(resp)) => resp,
        other => panic!("Expected ContractGetBytecodeResponse, got {:?}", other),
    };

    assert_eq!(
        bytecode_response
            .header
            .as_ref()
            .unwrap()
            .node_transaction_precheck_code,
        ResponseCodeEnum::Ok as i32
    );
    assert_eq!(bytecode_response.bytecode, bytecode);
    Ok(())
}

/// Test Case: Get Contract Info (Not Found)
/// Objective: Verify `getContractInfo` returns INVALID_CONTRACT_ID for a non-existent contract.
#[tokio::test]
#[serial]
async fn test_get_contract_info_not_found() -> Result<()> {
    let ctx = TestContext::new().await?;
    let mut query_client = ctx.contract_client().await?;

    let contract_id = ContractId {
        contract: Some(contract_id::Contract::ContractNum(9999)),
        ..Default::default()
    };
    let query = TopLevelQuery {
        query: Some(Query::ContractGetInfo(ContractGetInfoQuery {
            header: None,
            contract_id: Some(contract_id),
        })),
    };

    let response = query_client.get_contract_info(query).await?.into_inner();
    let contract_response = match response.response {
        Some(response::Response::ContractGetInfo(resp)) => resp,
        other => panic!("Expected ContractGetInfo response, got {:?}", other),
    };

    assert_eq!(
        contract_response
            .header
            .as_ref()
            .unwrap()
            .node_transaction_precheck_code,
        ResponseCodeEnum::InvalidContractId as i32
    );
    assert!(contract_response.contract_info.is_none());
    Ok(())
}
