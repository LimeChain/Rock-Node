use rock_node_core::StateReader;
use rock_node_protobufs::proto::{
    consensus_service_server::ConsensusService, Query, Response as TopLevelResponse,
    ResponseCodeEnum, Transaction, TransactionResponse,
};
use std::sync::Arc;
use tonic::{Request, Response, Status};

/// The gRPC server implementation for the HAPI `ConsensusService`.
#[derive(Debug)]
pub struct ConsensusServiceImpl {
    pub state_reader: Arc<dyn StateReader>,
}

impl ConsensusServiceImpl {
    pub fn new(state_reader: Arc<dyn StateReader>) -> Self {
        Self { state_reader }
    }
}

#[tonic::async_trait]
impl ConsensusService for ConsensusServiceImpl {
    // QUERIES (Not Implemented)
    async fn get_topic_info(
        &self,
        _request: Request<Query>,
    ) -> Result<Response<TopLevelResponse>, Status> {
        Err(Status::unimplemented("Query not yet implemented"))
    }

    // TRANSACTIONS (Not Supported)
    async fn create_topic(
        &self,
        _request: Request<Transaction>,
    ) -> Result<Response<TransactionResponse>, Status> {
        Ok(Response::new(TransactionResponse {
            node_transaction_precheck_code: ResponseCodeEnum::NotSupported as i32,
            cost: 0,
        }))
    }
    async fn update_topic(
        &self,
        _request: Request<Transaction>,
    ) -> Result<Response<TransactionResponse>, Status> {
        Ok(Response::new(TransactionResponse {
            node_transaction_precheck_code: ResponseCodeEnum::NotSupported as i32,
            cost: 0,
        }))
    }
    async fn delete_topic(
        &self,
        _request: Request<Transaction>,
    ) -> Result<Response<TransactionResponse>, Status> {
        Ok(Response::new(TransactionResponse {
            node_transaction_precheck_code: ResponseCodeEnum::NotSupported as i32,
            cost: 0,
        }))
    }
    async fn submit_message(
        &self,
        _request: Request<Transaction>,
    ) -> Result<Response<TransactionResponse>, Status> {
        Ok(Response::new(TransactionResponse {
            node_transaction_precheck_code: ResponseCodeEnum::NotSupported as i32,
            cost: 0,
        }))
    }
}
