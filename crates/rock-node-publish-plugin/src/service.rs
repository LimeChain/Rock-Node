use crate::{session_manager::SessionManager, state::SharedState};
use rock_node_core::AppContext;
use rock_node_protobufs::org::hiero::block::api::{
    block_stream_publish_service_server::BlockStreamPublishService, PublishStreamRequest,
    PublishStreamResponse,
};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};
use tracing::info;

/// Implements the gRPC service.
#[derive(Debug)]
pub struct PublishServiceImpl {
    pub context: AppContext,
    pub shared_state: Arc<SharedState>,
}

#[tonic::async_trait]
impl BlockStreamPublishService for PublishServiceImpl {
    type publishBlockStreamStream = ReceiverStream<Result<PublishStreamResponse, Status>>;

    async fn publish_block_stream(
        &self,
        request: Request<tonic::Streaming<PublishStreamRequest>>,
    ) -> Result<Response<Self::publishBlockStreamStream>, Status> {
        let mut inbound_stream = request.into_inner();
        let (response_tx, response_rx) = mpsc::channel(16);

        let mut session_manager =
            SessionManager::new(self.context.clone(), self.shared_state.clone(), response_tx);
        let session_id = session_manager.id;
        info!(%session_id, "New publisher connection. Spawning handler task.");

        tokio::spawn(async move {
            while let Some(request_result) = inbound_stream.message().await.ok().flatten() {
                if let Some(request_type) = request_result.request {
                    if session_manager.handle_request(request_type).await {
                        break;
                    }
                }
            }
            info!(%session_id, "Session handler task finished.");
        });

        Ok(Response::new(ReceiverStream::new(response_rx)))
    }
}
