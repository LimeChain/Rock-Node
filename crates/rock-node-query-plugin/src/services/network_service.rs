use crate::handlers::network_handler::NetworkQueryHandler;
use rock_node_core::StateReader;
use rock_node_protobufs::proto::{
    network_service_server::NetworkService, query, response, Query, Response as TopLevelResponse,
    ResponseCodeEnum, Transaction, TransactionResponse,
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
    async fn get_version_info(
        &self,
        request: Request<Query>,
    ) -> Result<Response<TopLevelResponse>, Status> {
        if let Some(query::Query::NetworkGetVersionInfo(q)) = request.into_inner().query {
            let handler = NetworkQueryHandler::new(self.state_reader.clone());
            let specific_response = handler.get_version_info(q).await?;
            let top_level_response = TopLevelResponse {
                response: Some(response::Response::NetworkGetVersionInfo(specific_response)),
            };
            Ok(Response::new(top_level_response))
        } else {
            Err(Status::invalid_argument(
                "Incorrect query type provided for getVersionInfo",
            ))
        }
    }

    async fn get_execution_time(
        &self,
        _request: Request<Query>,
    ) -> Result<Response<TopLevelResponse>, Status> {
        Err(Status::unimplemented("getExecutionTime is deprecated"))
    }

    async fn get_account_details(
        &self,
        request: Request<Query>,
    ) -> Result<Response<TopLevelResponse>, Status> {
        if let Some(query::Query::AccountDetails(q)) = request.into_inner().query {
            let handler = NetworkQueryHandler::new(self.state_reader.clone());
            let specific_response = handler.get_account_details(q).await?;
            let top_level_response = TopLevelResponse {
                response: Some(response::Response::AccountDetails(specific_response)),
            };
            Ok(Response::new(top_level_response))
        } else {
            Err(Status::invalid_argument(
                "Incorrect query type provided for getAccountDetails",
            ))
        }
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
