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
