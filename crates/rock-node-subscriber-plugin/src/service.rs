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
use tracing::{info, warn};

/// Implements the gRPC service for subscribing to block streams.
#[derive(Debug)]
pub struct SubscriberServiceImpl {
    context: Arc<AppContext>,
}

impl SubscriberServiceImpl {
    pub fn new(context: Arc<AppContext>) -> Self {
        Self {
            context
        }
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
        info!(?request, ?remote_addr, "Received new subscribeBlockStream request.");

        // Create a channel for this specific client session. The session task will send
        // responses into the `tx` side, and `tonic` will serve them from the `rx` side.
        let (tx, rx) = mpsc::channel(16); // Small buffer is fine, backpressure is handled inside the session.

        // Create a new session to handle the logic for this specific client.
        let mut session = SubscriberSession::new(self.context.clone(), request, tx);

        // Spawn a new asynchronous task to run the session.
        // This isolates each client and allows the server to handle many concurrent connections.
        tokio::spawn(async move {
            info!(session_id = %session.id, "Spawning new session handler task.");
            if let Err(e) = session.run().await {
                warn!(session_id = %session.id, error = ?e, "Session task ended with an error.");
            } else {
                info!(session_id = %session.id, "Session task completed successfully.");
            }
        });

        // Return the receiver stream to Tonic. Tonic will poll this stream and send
        // any messages it receives to the client.
        Ok(Response::new(ReceiverStream::new(rx)))
    }
}
