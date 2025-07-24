use rock_node_core::StateReader;
use rock_node_protobufs::proto::{
    schedule_service_server::ScheduleService, Query, Response as TopLevelResponse,
    ResponseCodeEnum, Transaction, TransactionResponse,
};
use std::sync::Arc;
use tonic::{Request, Response, Status};

/// The gRPC server implementation for the HAPI `ScheduleService`.
#[derive(Debug)]
pub struct ScheduleServiceImpl {
    pub state_reader: Arc<dyn StateReader>,
}

impl ScheduleServiceImpl {
    pub fn new(state_reader: Arc<dyn StateReader>) -> Self {
        Self { state_reader }
    }
}

#[tonic::async_trait]
impl ScheduleService for ScheduleServiceImpl {
    // QUERIES (Not Implemented)
    async fn get_schedule_info(
        &self,
        _request: Request<Query>,
    ) -> Result<Response<TopLevelResponse>, Status> {
        Err(Status::unimplemented("Query not yet implemented"))
    }

    // TRANSACTIONS (Not Supported)
    async fn create_schedule(
        &self,
        _request: Request<Transaction>,
    ) -> Result<Response<TransactionResponse>, Status> {
        Ok(Response::new(TransactionResponse {
            node_transaction_precheck_code: ResponseCodeEnum::NotSupported as i32,
            cost: 0,
        }))
    }
    async fn sign_schedule(
        &self,
        _request: Request<Transaction>,
    ) -> Result<Response<TransactionResponse>, Status> {
        Ok(Response::new(TransactionResponse {
            node_transaction_precheck_code: ResponseCodeEnum::NotSupported as i32,
            cost: 0,
        }))
    }
    async fn delete_schedule(
        &self,
        _request: Request<Transaction>,
    ) -> Result<Response<TransactionResponse>, Status> {
        Ok(Response::new(TransactionResponse {
            node_transaction_precheck_code: ResponseCodeEnum::NotSupported as i32,
            cost: 0,
        }))
    }
}
