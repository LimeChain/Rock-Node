use rock_node_core::StateReader;
use rock_node_protobufs::proto::{
    network_service_server::NetworkService, Query, Response as TopLevelResponse, ResponseCodeEnum,
    Transaction, TransactionResponse,
};
use std::sync::Arc;
use tonic::{Request, Response, Status};

/// The gRPC server implementation for the HAPI `NetworkService`.
#[derive(Debug)]
pub struct NetworkServiceImpl {
    pub state_reader: Arc<dyn StateReader>,
}

impl NetworkServiceImpl {
    pub fn new(state_reader: Arc<dyn StateReader>) -> Self {
        Self { state_reader }
    }
}

#[tonic::async_trait]
impl NetworkService for NetworkServiceImpl {
    // QUERIES (Not Implemented)
    async fn get_version_info(
        &self,
        _request: Request<Query>,
    ) -> Result<Response<TopLevelResponse>, Status> {
        Err(Status::unimplemented("Query not yet implemented"))
    }

    async fn get_execution_time(
        &self,
        _request: Request<Query>,
    ) -> Result<Response<TopLevelResponse>, Status> {
        Err(Status::unimplemented("Query not yet implemented"))
    }

    async fn get_account_details(
        &self,
        _request: Request<Query>,
    ) -> Result<Response<TopLevelResponse>, Status> {
        Err(Status::unimplemented("Query not yet implemented"))
    }

    // TRANSACTIONS (Not Supported)
    async fn unchecked_submit(
        &self,
        _request: Request<Transaction>,
    ) -> Result<Response<TransactionResponse>, Status> {
        Ok(Response::new(TransactionResponse {
            node_transaction_precheck_code: ResponseCodeEnum::NotSupported as i32,
            cost: 0,
        }))
    }
}
