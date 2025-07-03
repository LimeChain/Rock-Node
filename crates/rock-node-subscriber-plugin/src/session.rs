use crate::error::SubscriberError;
use rock_node_core::app_context::AppContext;
use rock_node_protobufs::org::hiero::block::api::{SubscribeStreamRequest, SubscribeStreamResponse};
use std::sync::Arc;
use tokio::sync::mpsc;
use tonic::Status;
use uuid::Uuid;

/// Manages the state and logic for a single client subscription stream.
pub struct SubscriberSession {
    pub id: Uuid,
    context: Arc<AppContext>,
    request: SubscribeStreamRequest,
    response_tx: mpsc::Sender<Result<SubscribeStreamResponse, Status>>,
}

impl SubscriberSession {
    /// Creates a new subscriber session.
    pub fn new(
        context: Arc<AppContext>,
        request: SubscribeStreamRequest,
        response_tx: mpsc::Sender<Result<SubscribeStreamResponse, Status>>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            context,
            request,
            response_tx,
        }
    }

    /// Runs the entire lifecycle of the subscription stream.
    pub async fn run(&mut self) -> Result<(), SubscriberError> {
        // Step 3 will implement the logic here:
        // 1. Increment active session metric.
        // 2. Validate the request.
        // 3. Perform historical streaming.
        // 4. Transition to live streaming with backpressure.
        // 5. Decrement metric in a Drop guard.

        // For now, just send a success message and close.
        let response = SubscribeStreamResponse {
            response: Some(
                rock_node_protobufs::org::hiero::block::api::subscribe_stream_response::Response::Status(
                    rock_node_protobufs::org::hiero::block::api::subscribe_stream_response::Code::ReadStreamSuccess.into(),
                ),
            ),
        };

        if self.response_tx.send(Ok(response)).await.is_err() {
            // Client disconnected before we could even send the success message.
            return Err(SubscriberError::ClientDisconnected);
        }

        Ok(())
    }
}
