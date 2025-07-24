use crate::common::{block_builder::BlockBuilder, TestContext};
use anyhow::Result;
use prost::Message;
use rock_node_protobufs::{
    org::hiero::block::api::{
        publish_stream_request::Request as PublishRequest, BlockItemSet, PublishStreamRequest,
    },
    proto::{
        account_id::Account as AccountIdType, account_id::Account,
        crypto_get_account_balance_query, query::Query, response, AccountId,
        CryptoGetAccountBalanceQuery, CryptoGetAccountRecordsQuery, CryptoGetInfoQuery,
        Query as TopLevelQuery, ResponseCodeEnum, TransactionGetReceiptQuery,
        TransactionGetRecordQuery, TransactionId,
    },
};
use serial_test::serial;
use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, StreamExt};

/// Test Case: Get Account Info
/// Objective: Verify that the `getAccountInfo` query returns the correct account info
/// for an account after its state has been updated.
#[tokio::test]
#[serial]
async fn test_get_account_info_successfully() -> Result<()> {
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

/// Test Case: Get Account Balance
/// Objective: Verify that the `cryptoGetBalance` query returns the correct balance
/// for an account after its state has been updated.
#[tokio::test]
#[serial]
async fn test_get_account_balance_successfully() -> Result<()> {
    let ctx = TestContext::new().await?;
    let mut publish_client = ctx.publisher_client().await?;
    let mut query_client = ctx.query_client().await?;

    // 1. Build and publish a block that sets the balance for account 0.0.1001
    let block_bytes = BlockBuilder::new(0)
        .with_account_state_change(1001, "balance-test")
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

    // 2. Query for the account's balance
    let account_id = AccountId {
        shard_num: 0,
        realm_num: 0,
        account: Some(AccountIdType::AccountNum(1001)),
    };
    let query = TopLevelQuery {
        query: Some(Query::CryptogetAccountBalance(
            CryptoGetAccountBalanceQuery {
                header: None,
                balance_source: Some(crypto_get_account_balance_query::BalanceSource::AccountId(
                    account_id,
                )),
            },
        )),
    };

    let response = query_client.crypto_get_balance(query).await?.into_inner();

    // 3. Assert the response is correct
    let balance_response = match response.response {
        Some(response::Response::CryptogetAccountBalance(resp)) => resp,
        other => panic!("Expected CryptoGetAccountBalanceResponse, got {:?}", other),
    };

    assert_eq!(
        balance_response
            .header
            .as_ref()
            .unwrap()
            .node_transaction_precheck_code,
        ResponseCodeEnum::Ok as i32
    );
    // The balance is set to the account number in the builder for simplicity
    assert_eq!(balance_response.balance, 1001);

    Ok(())
}

/// Test Case: Get Account Records (Not Implemented)
/// Objective: Verify that the `getAccountRecords` query returns an OK status
/// with an empty list, as per the current implementation.
#[tokio::test]
#[serial]
async fn test_get_account_records_returns_empty() -> Result<()> {
    let ctx = TestContext::new().await?;
    let mut query_client = ctx.query_client().await?;

    let account_id = AccountId {
        shard_num: 0,
        realm_num: 0,
        account: Some(Account::AccountNum(1002)),
    };
    let query = TopLevelQuery {
        query: Some(Query::CryptoGetAccountRecords(
            CryptoGetAccountRecordsQuery {
                header: None,
                account_id: Some(account_id),
            },
        )),
    };

    let response = query_client.get_account_records(query).await?.into_inner();

    let records_response = match response.response {
        Some(response::Response::CryptoGetAccountRecords(resp)) => resp,
        other => panic!("Expected CryptoGetAccountRecordsResponse, got {:?}", other),
    };

    assert_eq!(
        records_response
            .header
            .as_ref()
            .unwrap()
            .node_transaction_precheck_code,
        ResponseCodeEnum::Ok as i32
    );
    assert!(records_response.records.is_empty());

    Ok(())
}

/// Test Case: Get Transaction Receipt (Not Found)
/// Objective: Verify that `getTransactionReceipts` returns RECEIPT_NOT_FOUND.
#[tokio::test]
#[serial]
async fn test_get_transaction_receipt_not_found() -> Result<()> {
    let ctx = TestContext::new().await?;
    let mut query_client = ctx.query_client().await?;

    let transaction_id = TransactionId {
        transaction_valid_start: None,
        account_id: Some(AccountId {
            shard_num: 0,
            realm_num: 0,
            account: Some(Account::AccountNum(2)),
        }),
        scheduled: false,
        nonce: 0,
    };
    let query = TopLevelQuery {
        query: Some(Query::TransactionGetReceipt(TransactionGetReceiptQuery {
            header: None,
            transaction_id: Some(transaction_id),
            include_duplicates: false,
            include_child_receipts: false,
        })),
    };

    let response = query_client
        .get_transaction_receipts(query)
        .await?
        .into_inner();

    let receipt_response = match response.response {
        Some(response::Response::TransactionGetReceipt(resp)) => resp,
        other => panic!("Expected TransactionGetReceiptResponse, got {:?}", other),
    };

    assert_eq!(
        receipt_response
            .header
            .as_ref()
            .unwrap()
            .node_transaction_precheck_code,
        ResponseCodeEnum::ReceiptNotFound as i32
    );

    Ok(())
}

/// Test Case: Get Transaction Record (Not Found)
/// Objective: Verify that `getTxRecordByTxID` returns RECORD_NOT_FOUND.
#[tokio::test]
#[serial]
async fn test_get_transaction_record_not_found() -> Result<()> {
    let ctx = TestContext::new().await?;
    let mut query_client = ctx.query_client().await?;

    let transaction_id = TransactionId {
        transaction_valid_start: None,
        account_id: Some(AccountId {
            shard_num: 0,
            realm_num: 0,
            account: Some(Account::AccountNum(2)),
        }),
        scheduled: false,
        nonce: 0,
    };
    let query = TopLevelQuery {
        query: Some(Query::TransactionGetRecord(TransactionGetRecordQuery {
            header: None,
            transaction_id: Some(transaction_id),
            include_duplicates: false,
            include_child_records: false,
        })),
    };

    let response = query_client
        .get_tx_record_by_tx_id(query)
        .await?
        .into_inner();

    let record_response = match response.response {
        Some(response::Response::TransactionGetRecord(resp)) => resp,
        other => panic!("Expected TransactionGetRecordResponse, got {:?}", other),
    };

    assert_eq!(
        record_response
            .header
            .as_ref()
            .unwrap()
            .node_transaction_precheck_code,
        ResponseCodeEnum::RecordNotFound as i32
    );

    Ok(())
}
