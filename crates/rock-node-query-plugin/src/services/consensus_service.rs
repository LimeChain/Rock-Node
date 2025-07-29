use crate::handlers::consensus_handler::ConsensusQueryHandler;
use rock_node_core::StateReader;
use rock_node_protobufs::proto::{
    consensus_service_server::ConsensusService, query, response, Query,
    Response as TopLevelResponse, ResponseCodeEnum, Transaction, TransactionResponse,
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
    async fn get_topic_info(
        &self,
        request: Request<Query>,
    ) -> Result<Response<TopLevelResponse>, Status> {
        if let Some(query::Query::ConsensusGetTopicInfo(q)) = request.into_inner().query {
            let handler = ConsensusQueryHandler::new(self.state_reader.clone());
            let specific_response = handler.get_topic_info(q).await?;
            let top_level_response = TopLevelResponse {
                response: Some(response::Response::ConsensusGetTopicInfo(specific_response)),
            };
            Ok(Response::new(top_level_response))
        } else {
            Err(Status::invalid_argument(
                "Incorrect query type provided for getTopicInfo",
            ))
        }
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
