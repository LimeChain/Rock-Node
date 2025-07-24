use rock_node_core::StateReader;
use rock_node_protobufs::proto::{
    file_service_server::FileService, Query, Response as TopLevelResponse, ResponseCodeEnum,
    Transaction, TransactionResponse,
};
use std::sync::Arc;
use tonic::{Request, Response, Status};

/// The gRPC server implementation for the HAPI `FileService`.
#[derive(Debug)]
pub struct FileServiceImpl {
    pub state_reader: Arc<dyn StateReader>,
}

impl FileServiceImpl {
    pub fn new(state_reader: Arc<dyn StateReader>) -> Self {
        Self { state_reader }
    }
}

#[tonic::async_trait]
impl FileService for FileServiceImpl {
    // QUERY (Not Implemented)
    async fn get_file_content(
        &self,
        _request: Request<Query>,
    ) -> Result<Response<TopLevelResponse>, Status> {
        Err(Status::unimplemented("Query not yet implemented"))
    }

    async fn get_file_info(
        &self,
        _request: Request<Query>,
    ) -> Result<Response<TopLevelResponse>, Status> {
        Err(Status::unimplemented("Query not yet implemented"))
    }

    // TRANSACTIONS (Not Supported)
    async fn create_file(
        &self,
        _request: Request<Transaction>,
    ) -> Result<Response<TransactionResponse>, Status> {
        Ok(Response::new(TransactionResponse {
            node_transaction_precheck_code: ResponseCodeEnum::NotSupported as i32,
            cost: 0,
        }))
    }
    async fn update_file(
        &self,
        _request: Request<Transaction>,
    ) -> Result<Response<TransactionResponse>, Status> {
        Ok(Response::new(TransactionResponse {
            node_transaction_precheck_code: ResponseCodeEnum::NotSupported as i32,
            cost: 0,
        }))
    }
    async fn delete_file(
        &self,
        _request: Request<Transaction>,
    ) -> Result<Response<TransactionResponse>, Status> {
        Ok(Response::new(TransactionResponse {
            node_transaction_precheck_code: ResponseCodeEnum::NotSupported as i32,
            cost: 0,
        }))
    }
    async fn append_content(
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
}
