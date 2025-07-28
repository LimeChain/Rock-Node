use crate::handlers::schedule_handler::ScheduleQueryHandler;
use rock_node_core::StateReader;
use rock_node_protobufs::proto::{
    query, response, schedule_service_server::ScheduleService, Query, Response as TopLevelResponse,
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
    async fn get_schedule_info(
        &self,
        request: Request<Query>,
    ) -> Result<Response<TopLevelResponse>, Status> {
        if let Some(query::Query::ScheduleGetInfo(q)) = request.into_inner().query {
            let handler = ScheduleQueryHandler::new(self.state_reader.clone());
            let specific_response = handler.get_schedule_info(q).await?;
            let top_level_response = TopLevelResponse {
                response: Some(response::Response::ScheduleGetInfo(specific_response)),
            };
            Ok(Response::new(top_level_response))
        } else {
            Err(Status::invalid_argument(
                "Incorrect query type provided for getScheduleInfo",
            ))
        }
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
