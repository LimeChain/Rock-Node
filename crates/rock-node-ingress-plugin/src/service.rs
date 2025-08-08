use crate::{consensus_manager::ConsensusSession, state::SharedState};
use rock_node_core::{app_context::AppContext, events::BlockItemsReceived};
use rock_node_protobufs::org::hiero::block::api::{
    block_stream_publish_service_server::BlockStreamPublishService, PublishStreamRequest,
    PublishStreamResponse,
};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};
use tracing::info;

/// Implements the gRPC service.
#[derive(Debug)]
pub struct IngressServiceImpl {
    pub context: AppContext,
    pub shared_state: Arc<SharedState>,
    pub internal_tx: broadcast::Sender<BlockItemsReceived>,
}

#[tonic::async_trait]
impl BlockStreamPublishService for IngressServiceImpl {
    type publishBlockStreamStream = ReceiverStream<Result<PublishStreamResponse, Status>>;

    async fn publish_block_stream(
        &self,
        request: Request<tonic::Streaming<PublishStreamRequest>>,
    ) -> Result<Response<Self::publishBlockStreamStream>, Status> {
        let mut inbound_stream = request.into_inner();
        let (response_tx, response_rx) = mpsc::channel(16);

        self.context.metrics.active_publish_sessions.inc();
        let metrics_clone = self.context.metrics.clone();

        let mut session = ConsensusSession::new(
            self.context.clone(),
            self.shared_state.clone(),
            self.internal_tx.clone(),
            response_tx.clone(),
        );
        let session_id = session.id;
        info!(%session_id, "New publisher connection. Spawning consensus session task.");

        self.shared_state
            .active_sessions
            .insert(session_id, response_tx);
        let shared_state_clone = self.shared_state.clone();

        tokio::spawn(async move {
            while let Some(request_result) = inbound_stream.message().await.ok().flatten() {
                if let Some(request_type) = request_result.request {
                    if session.handle_request(request_type).await {
                        break;
                    }
                }
            }
            info!(%session_id, "Consensus session task finished.");

            use rock_node_protobufs::org::hiero::block::api::{
                publish_stream_response::{ResendBlock, Response},
                PublishStreamResponse,
            };

            let mut affected_blocks = Vec::new();
            for entry in shared_state_clone.block_winners.iter() {
                if *entry.value() == session_id {
                    affected_blocks.push(*entry.key());
                }
            }

            for block_number in affected_blocks {
                shared_state_clone.block_winners.remove(&block_number);

                let resend_msg = PublishStreamResponse {
                    response: Some(Response::ResendBlock(ResendBlock { block_number })),
                };

                for session_entry in shared_state_clone.active_sessions.iter() {
                    if *session_entry.key() != session_id {
                        let _ = session_entry.value().send(Ok(resend_msg.clone())).await;
                    }
                }

                metrics_clone
                    .publish_responses_sent_total
                    .with_label_values(&["ResendBlock"])
                    .inc();
            }

            shared_state_clone.active_sessions.remove(&session_id);
            metrics_clone.active_publish_sessions.dec();
        });

        Ok(Response::new(ReceiverStream::new(response_rx)))
    }
}
