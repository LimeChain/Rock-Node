use crate::session::SubscriberSession;
use rock_node_core::app_context::AppContext;
use rock_node_protobufs::org::hiero::block::api::{
    block_stream_subscribe_service_server::BlockStreamSubscribeService, SubscribeStreamRequest,
    SubscribeStreamResponse,
};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};
use tracing::info;

/// Implements the gRPC service for subscribing to block streams.
#[derive(Debug)]
pub struct SubscriberServiceImpl {
    context: Arc<AppContext>,
}

impl SubscriberServiceImpl {
    pub fn new(context: Arc<AppContext>) -> Self {
        Self { context }
    }
}

#[tonic::async_trait]
impl BlockStreamSubscribeService for SubscriberServiceImpl {
    type subscribeBlockStreamStream = ReceiverStream<Result<SubscribeStreamResponse, Status>>;

    async fn subscribe_block_stream(
        &self,
        request: Request<SubscribeStreamRequest>,
    ) -> Result<Response<Self::subscribeBlockStreamStream>, Status> {
        let remote_addr = request.remote_addr();
        let request = request.into_inner();
        info!(
            ?request,
            ?remote_addr,
            "Received new subscribeBlockStream request."
        );

        let (tx, rx) = mpsc::channel(16);

        let mut session = SubscriberSession::new(self.context.clone(), request, tx);

        tokio::spawn(async move {
            info!(session_id = %session.id, "Spawning new session handler task.");
            session.run().await;
            info!(session_id = %session.id, "Session handler task finished.");
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }
}
