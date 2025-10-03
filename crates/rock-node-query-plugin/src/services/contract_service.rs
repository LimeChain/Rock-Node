use rock_node_core::StateReader;
use rock_node_protobufs::proto::{
    query, response, smart_contract_service_server::SmartContractService, Query,
    Response as TopLevelResponse, ResponseCodeEnum, Transaction, TransactionResponse,
};
use std::sync::Arc;
use tonic::{Request, Response, Status};

use crate::handlers::contract_handler::ContractQueryHandler;

/// The gRPC server implementation for the HAPI `SmartContractService`.
#[derive(Debug)]
pub struct SmartContractServiceImpl {
    pub state_reader: Arc<dyn StateReader>,
}

impl SmartContractServiceImpl {
    pub fn new(state_reader: Arc<dyn StateReader>) -> Self {
        Self { state_reader }
    }
}

#[tonic::async_trait]
impl SmartContractService for SmartContractServiceImpl {
    // QUERIES
    async fn contract_call_local_method(
        &self,
        _request: Request<Query>,
    ) -> Result<Response<TopLevelResponse>, Status> {
        Err(Status::unimplemented("Query not yet implemented"))
    }

    async fn get_contract_info(
        &self,
        request: Request<Query>,
    ) -> Result<Response<TopLevelResponse>, Status> {
        if let Some(query::Query::ContractGetInfo(q)) = request.into_inner().query {
            let handler = ContractQueryHandler::new(self.state_reader.clone());
            let specific_response = handler.get_contract_info(q).await?;
            let top_level_response = TopLevelResponse {
                response: Some(response::Response::ContractGetInfo(specific_response)),
            };
            Ok(Response::new(top_level_response))
        } else {
            Err(Status::invalid_argument(
                "Incorrect query type provided for getContractInfo",
            ))
        }
    }

    async fn contract_get_bytecode(
        &self,
        request: Request<Query>,
    ) -> Result<Response<TopLevelResponse>, Status> {
        if let Some(query::Query::ContractGetBytecode(q)) = request.into_inner().query {
            let handler = ContractQueryHandler::new(self.state_reader.clone());
            let specific_response = handler.get_contract_bytecode(q).await?;
            let top_level_response = TopLevelResponse {
                response: Some(response::Response::ContractGetBytecodeResponse(
                    specific_response,
                )),
            };
            Ok(Response::new(top_level_response))
        } else {
            Err(Status::invalid_argument(
                "Incorrect query type provided for contractGetBytecode",
            ))
        }
    }

    async fn get_by_solidity_id(
        &self,
        _request: Request<Query>,
    ) -> Result<Response<TopLevelResponse>, Status> {
        Err(Status::unimplemented("Query not yet implemented"))
    }
    async fn get_tx_record_by_contract_id(
        &self,
        _request: Request<Query>,
    ) -> Result<Response<TopLevelResponse>, Status> {
        Err(Status::unimplemented("Query not yet implemented"))
    }

    // TRANSACTIONS (Not Supported)
    async fn create_contract(
        &self,
        _request: Request<Transaction>,
    ) -> Result<Response<TransactionResponse>, Status> {
        Ok(Response::new(TransactionResponse {
            node_transaction_precheck_code: ResponseCodeEnum::NotSupported as i32,
            cost: 0,
        }))
    }
    async fn update_contract(
        &self,
        _request: Request<Transaction>,
    ) -> Result<Response<TransactionResponse>, Status> {
        Ok(Response::new(TransactionResponse {
            node_transaction_precheck_code: ResponseCodeEnum::NotSupported as i32,
            cost: 0,
        }))
    }
    async fn contract_call_method(
        &self,
        _request: Request<Transaction>,
    ) -> Result<Response<TransactionResponse>, Status> {
        Ok(Response::new(TransactionResponse {
            node_transaction_precheck_code: ResponseCodeEnum::NotSupported as i32,
            cost: 0,
        }))
    }
    async fn delete_contract(
        &self,
        _request: Request<Transaction>,
    ) -> Result<Response<TransactionResponse>, Status> {
        Ok(Response::new(TransactionResponse {
            node_transaction_precheck_code: ResponseCodeEnum::NotSupported as i32,
            cost: 0,
        }))
    }
    async fn system_delete(
        &self,
        _request: Request<Transaction>,
    ) -> Result<Response<TransactionResponse>, Status> {
        Ok(Response::new(TransactionResponse {
            node_transaction_precheck_code: ResponseCodeEnum::NotSupported as i32,
            cost: 0,
        }))
    }
    async fn system_undelete(
        &self,
        _request: Request<Transaction>,
    ) -> Result<Response<TransactionResponse>, Status> {
        Ok(Response::new(TransactionResponse {
            node_transaction_precheck_code: ResponseCodeEnum::NotSupported as i32,
            cost: 0,
        }))
    }
    async fn call_ethereum(
        &self,
        _request: Request<Transaction>,
    ) -> Result<Response<TransactionResponse>, Status> {
        Ok(Response::new(TransactionResponse {
            node_transaction_precheck_code: ResponseCodeEnum::NotSupported as i32,
            cost: 0,
        }))
    }
    async fn lambda_s_store(
        &self,
        _request: Request<Transaction>,
    ) -> Result<Response<TransactionResponse>, Status> {
        Ok(Response::new(TransactionResponse {
            node_transaction_precheck_code: ResponseCodeEnum::NotSupported as i32,
            cost: 0,
        }))
    }
}
