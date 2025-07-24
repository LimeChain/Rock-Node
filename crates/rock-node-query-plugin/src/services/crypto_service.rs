use crate::handlers::crypto_handler::CryptoQueryHandler;
use rock_node_core::StateReader;
use rock_node_protobufs::proto::{
    crypto_service_server::CryptoService, query, response, Query, Response as TopLevelResponse,
    ResponseCodeEnum, Transaction, TransactionResponse,
};
use std::sync::Arc;
use tonic::{Request, Response, Status};

/// The gRPC server implementation for the HAPI `CryptoService`.
#[derive(Debug)]
pub struct CryptoServiceImpl {
    state_reader: Arc<dyn StateReader>,
}

impl CryptoServiceImpl {
    pub fn new(state_reader: Arc<dyn StateReader>) -> Self {
        Self { state_reader }
    }
}

#[tonic::async_trait]
impl CryptoService for CryptoServiceImpl {
    /// Get the account info for a given account_id.
    ///
    /// # Arguments
    ///
    /// * `request` - The request containing the query
    ///
    /// # Returns
    ///
    /// * `Response<TopLevelResponse>` - The response containing the account info
    async fn get_account_info(
        &self,
        request: Request<rock_node_protobufs::proto::Query>,
    ) -> Result<Response<TopLevelResponse>, Status> {
        if let Some(query::Query::CryptoGetInfo(q)) = request.into_inner().query {
            let handler = CryptoQueryHandler::new(self.state_reader.clone());
            let specific_response = handler.get_account_info(q).await?;

            // Package the specific response into the generic top-level Response.
            let top_level_response = TopLevelResponse {
                response: Some(response::Response::CryptoGetInfo(specific_response)),
            };
            Ok(Response::new(top_level_response))
        } else {
            Err(Status::invalid_argument(
                "Incorrect query type provided for getAccountInfo",
            ))
        }
    }

    async fn get_account_records(
        &self,
        request: Request<Query>,
    ) -> Result<Response<TopLevelResponse>, Status> {
        if let Some(query::Query::CryptoGetAccountRecords(q)) = request.into_inner().query {
            let handler = CryptoQueryHandler::new(self.state_reader.clone());
            let specific_response = handler.get_account_records(q).await?;
            let top_level_response = TopLevelResponse {
                response: Some(response::Response::CryptoGetAccountRecords(
                    specific_response,
                )),
            };
            Ok(Response::new(top_level_response))
        } else {
            Err(Status::invalid_argument(
                "Incorrect query type provided for getAccountRecords",
            ))
        }
    }
    async fn crypto_get_balance(
        &self,
        request: Request<Query>,
    ) -> Result<Response<TopLevelResponse>, Status> {
        if let Some(query::Query::CryptogetAccountBalance(q)) = request.into_inner().query {
            let handler = CryptoQueryHandler::new(self.state_reader.clone());
            let specific_response = handler.get_account_balance(q).await?;
            let top_level_response = TopLevelResponse {
                response: Some(response::Response::CryptogetAccountBalance(
                    specific_response,
                )),
            };
            Ok(Response::new(top_level_response))
        } else {
            Err(Status::invalid_argument(
                "Incorrect query type provided for cryptoGetBalance",
            ))
        }
    }
    async fn get_transaction_receipts(
        &self,
        request: Request<Query>,
    ) -> Result<Response<TopLevelResponse>, Status> {
        if let Some(query::Query::TransactionGetReceipt(q)) = request.into_inner().query {
            let handler = CryptoQueryHandler::new(self.state_reader.clone());
            let specific_response = handler.get_transaction_receipt(q).await?;
            let top_level_response = TopLevelResponse {
                response: Some(response::Response::TransactionGetReceipt(specific_response)),
            };
            Ok(Response::new(top_level_response))
        } else {
            Err(Status::invalid_argument(
                "Incorrect query type provided for getTransactionReceipts",
            ))
        }
    }
    async fn get_tx_record_by_tx_id(
        &self,
        request: Request<Query>,
    ) -> Result<Response<TopLevelResponse>, Status> {
        if let Some(query::Query::TransactionGetRecord(q)) = request.into_inner().query {
            let handler = CryptoQueryHandler::new(self.state_reader.clone());
            let specific_response = handler.get_transaction_record(q).await?;
            let top_level_response = TopLevelResponse {
                response: Some(response::Response::TransactionGetRecord(specific_response)),
            };
            Ok(Response::new(top_level_response))
        } else {
            Err(Status::invalid_argument(
                "Incorrect query type provided for getTxRecordByTxID",
            ))
        }
    }

    // QUERY (Not Implemented)
    async fn get_live_hash(&self, _: Request<Query>) -> Result<Response<TopLevelResponse>, Status> {
        // As per the protobuf file, this query is obsolete.
        Err(Status::unimplemented("getLiveHash is obsolete"))
    }

    // TRANSACTIONS (Not Supported)
    async fn create_account(
        &self,
        _: Request<Transaction>,
    ) -> Result<Response<TransactionResponse>, Status> {
        Ok(Response::new(TransactionResponse {
            node_transaction_precheck_code: ResponseCodeEnum::NotSupported as i32,
            cost: 0,
        }))
    }
    async fn update_account(
        &self,
        _: Request<Transaction>,
    ) -> Result<Response<TransactionResponse>, Status> {
        Ok(Response::new(TransactionResponse {
            node_transaction_precheck_code: ResponseCodeEnum::NotSupported as i32,
            cost: 0,
        }))
    }
    async fn crypto_transfer(
        &self,
        _: Request<Transaction>,
    ) -> Result<Response<TransactionResponse>, Status> {
        Ok(Response::new(TransactionResponse {
            node_transaction_precheck_code: ResponseCodeEnum::NotSupported as i32,
            cost: 0,
        }))
    }
    async fn crypto_delete(
        &self,
        _: Request<Transaction>,
    ) -> Result<Response<TransactionResponse>, Status> {
        Ok(Response::new(TransactionResponse {
            node_transaction_precheck_code: ResponseCodeEnum::NotSupported as i32,
            cost: 0,
        }))
    }
    async fn approve_allowances(
        &self,
        _: Request<Transaction>,
    ) -> Result<Response<TransactionResponse>, Status> {
        Ok(Response::new(TransactionResponse {
            node_transaction_precheck_code: ResponseCodeEnum::NotSupported as i32,
            cost: 0,
        }))
    }
    async fn delete_allowances(
        &self,
        _: Request<Transaction>,
    ) -> Result<Response<TransactionResponse>, Status> {
        Ok(Response::new(TransactionResponse {
            node_transaction_precheck_code: ResponseCodeEnum::NotSupported as i32,
            cost: 0,
        }))
    }
    async fn add_live_hash(
        &self,
        _: Request<Transaction>,
    ) -> Result<Response<TransactionResponse>, Status> {
        Ok(Response::new(TransactionResponse {
            node_transaction_precheck_code: ResponseCodeEnum::NotSupported as i32,
            cost: 0,
        }))
    }
    async fn delete_live_hash(
        &self,
        _: Request<Transaction>,
    ) -> Result<Response<TransactionResponse>, Status> {
        Ok(Response::new(TransactionResponse {
            node_transaction_precheck_code: ResponseCodeEnum::NotSupported as i32,
            cost: 0,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;
    use rock_node_core::StateReader;
    use rock_node_protobufs::{
        com::hedera::hapi::block::stream::output::{
            map_change_key, map_change_value, MapChangeKey, MapChangeValue, StateIdentifier,
        },
        proto::{
            account_id, crypto_get_account_balance_query, Account, AccountId,
            CryptoGetAccountBalanceQuery, CryptoGetAccountRecordsQuery, CryptoGetInfoQuery,
            Query as TopLevelQuery, TransactionGetReceiptQuery, TransactionGetRecordQuery,
            TransactionId,
        },
    };
    use std::collections::HashMap;

    /// A mock implementation of `StateReader` for controlled testing.
    #[derive(Debug, Default)]
    struct MockStateReader {
        state: HashMap<Vec<u8>, Vec<u8>>,
    }

    impl MockStateReader {
        fn insert(&mut self, key: Vec<u8>, value: Vec<u8>) {
            self.state.insert(key, value);
        }
    }

    impl StateReader for MockStateReader {
        fn get_state_value(&self, key: &[u8]) -> anyhow::Result<Option<Vec<u8>>> {
            Ok(self.state.get(key).cloned())
        }
    }

    // Helper function to generate the database key, mirroring the handler's logic.
    fn generate_db_key(account_id: &AccountId) -> Vec<u8> {
        let state_id = StateIdentifier::StateIdAccounts as u32;
        let map_key = MapChangeKey {
            key_choice: Some(map_change_key::KeyChoice::AccountIdKey(account_id.clone())),
        };
        [state_id.to_be_bytes().as_slice(), &map_key.encode_to_vec()].concat()
    }

    #[tokio::test]
    async fn test_get_account_info_found() {
        let account_id = AccountId {
            shard_num: 0,
            realm_num: 0,
            account: Some(account_id::Account::AccountNum(1001)),
        };
        let account = Account {
            account_id: Some(account_id.clone()),
            memo: "test".to_string(),
            ..Default::default()
        };
        let map_value = MapChangeValue {
            value_choice: Some(map_change_value::ValueChoice::AccountValue(account)),
        };

        let mut mock_reader = MockStateReader::default();
        let key = generate_db_key(&account_id);
        mock_reader.insert(key, map_value.encode_to_vec());

        let service = CryptoServiceImpl::new(Arc::new(mock_reader));
        let request = Request::new(TopLevelQuery {
            query: Some(query::Query::CryptoGetInfo(CryptoGetInfoQuery {
                account_id: Some(account_id.clone()),
                header: None,
            })),
        });

        let response = service.get_account_info(request).await.unwrap();
        let inner = response.into_inner();
        match inner.response {
            Some(response::Response::CryptoGetInfo(info)) => {
                assert_eq!(
                    info.header.unwrap().node_transaction_precheck_code,
                    ResponseCodeEnum::Ok as i32
                );
                assert_eq!(info.account_info.unwrap().memo, "test");
            }
            _ => panic!("Incorrect response type"),
        }
    }

    #[tokio::test]
    async fn test_get_account_info_not_found() {
        let account_id = AccountId {
            shard_num: 0,
            realm_num: 0,
            account: Some(account_id::Account::AccountNum(1002)),
        };
        let mock_reader = MockStateReader::default();
        let service = CryptoServiceImpl::new(Arc::new(mock_reader));

        let request = Request::new(TopLevelQuery {
            query: Some(query::Query::CryptoGetInfo(CryptoGetInfoQuery {
                account_id: Some(account_id),
                header: None,
            })),
        });

        let response = service.get_account_info(request).await.unwrap();
        let inner = response.into_inner();
        match inner.response {
            Some(response::Response::CryptoGetInfo(info)) => {
                assert_eq!(
                    info.header.unwrap().node_transaction_precheck_code,
                    ResponseCodeEnum::InvalidAccountId as i32
                );
                assert!(info.account_info.is_none());
            }
            _ => panic!("Incorrect response type"),
        }
    }

    #[tokio::test]
    async fn test_get_account_balance_found() {
        let account_id = AccountId {
            shard_num: 0,
            realm_num: 0,
            account: Some(account_id::Account::AccountNum(1003)),
        };
        let account = Account {
            account_id: Some(account_id.clone()),
            tinybar_balance: 1_000_000,
            ..Default::default()
        };
        let map_value = MapChangeValue {
            value_choice: Some(map_change_value::ValueChoice::AccountValue(account)),
        };

        let mut mock_reader = MockStateReader::default();
        let key = generate_db_key(&account_id);
        mock_reader.insert(key, map_value.encode_to_vec());

        let service = CryptoServiceImpl::new(Arc::new(mock_reader));
        let request = Request::new(TopLevelQuery {
            query: Some(query::Query::CryptogetAccountBalance(
                CryptoGetAccountBalanceQuery {
                    header: None,
                    balance_source: Some(
                        crypto_get_account_balance_query::BalanceSource::AccountId(
                            account_id.clone(),
                        ),
                    ),
                },
            )),
        });

        let response = service.crypto_get_balance(request).await.unwrap();
        let inner = response.into_inner();
        match inner.response {
            Some(response::Response::CryptogetAccountBalance(balance_info)) => {
                assert_eq!(
                    balance_info.header.unwrap().node_transaction_precheck_code,
                    ResponseCodeEnum::Ok as i32
                );
                assert_eq!(balance_info.balance, 1_000_000);
            }
            _ => panic!("Incorrect response type"),
        }
    }

    #[tokio::test]
    async fn test_get_account_records_empty() {
        let mock_reader = MockStateReader::default();
        let service = CryptoServiceImpl::new(Arc::new(mock_reader));
        let request = Request::new(TopLevelQuery {
            query: Some(query::Query::CryptoGetAccountRecords(
                CryptoGetAccountRecordsQuery {
                    header: None,
                    account_id: Some(AccountId::default()),
                },
            )),
        });

        let response = service.get_account_records(request).await.unwrap();
        let inner = response.into_inner();
        match inner.response {
            Some(response::Response::CryptoGetAccountRecords(records_info)) => {
                assert_eq!(
                    records_info.header.unwrap().node_transaction_precheck_code,
                    ResponseCodeEnum::Ok as i32
                );
                assert!(records_info.records.is_empty());
            }
            _ => panic!("Incorrect response type"),
        }
    }

    #[tokio::test]
    async fn test_get_transaction_receipt_not_found() {
        let mock_reader = MockStateReader::default();
        let service = CryptoServiceImpl::new(Arc::new(mock_reader));
        let request = Request::new(TopLevelQuery {
            query: Some(query::Query::TransactionGetReceipt(
                TransactionGetReceiptQuery {
                    header: None,
                    transaction_id: Some(TransactionId::default()),
                    include_duplicates: false,
                    include_child_receipts: false,
                },
            )),
        });

        let response = service.get_transaction_receipts(request).await.unwrap();
        let inner = response.into_inner();
        match inner.response {
            Some(response::Response::TransactionGetReceipt(receipt_info)) => {
                assert_eq!(
                    receipt_info.header.unwrap().node_transaction_precheck_code,
                    ResponseCodeEnum::ReceiptNotFound as i32
                );
            }
            _ => panic!("Incorrect response type"),
        }
    }

    #[tokio::test]
    async fn test_get_transaction_record_not_found() {
        let mock_reader = MockStateReader::default();
        let service = CryptoServiceImpl::new(Arc::new(mock_reader));
        let request = Request::new(TopLevelQuery {
            query: Some(query::Query::TransactionGetRecord(
                TransactionGetRecordQuery {
                    header: None,
                    transaction_id: Some(TransactionId::default()),
                    include_duplicates: false,
                    include_child_records: false,
                },
            )),
        });

        let response = service.get_tx_record_by_tx_id(request).await.unwrap();
        let inner = response.into_inner();
        match inner.response {
            Some(response::Response::TransactionGetRecord(record_info)) => {
                assert_eq!(
                    record_info.header.unwrap().node_transaction_precheck_code,
                    ResponseCodeEnum::RecordNotFound as i32
                );
            }
            _ => panic!("Incorrect response type"),
        }
    }

    #[tokio::test]
    async fn test_unsupported_transactions() {
        let mock_reader = MockStateReader::default();
        let service = CryptoServiceImpl::new(Arc::new(mock_reader));

        let response = service
            .create_account(Request::new(Transaction::default()))
            .await
            .unwrap();
        assert_eq!(
            response.into_inner().node_transaction_precheck_code,
            ResponseCodeEnum::NotSupported as i32
        );

        let response = service
            .update_account(Request::new(Transaction::default()))
            .await
            .unwrap();
        assert_eq!(
            response.into_inner().node_transaction_precheck_code,
            ResponseCodeEnum::NotSupported as i32
        );
    }
}
