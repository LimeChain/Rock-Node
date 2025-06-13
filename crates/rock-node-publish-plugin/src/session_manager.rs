use crate::state::{SharedState, SessionState};
use anyhow::Result;
use rock_node_core::AppContext;
use rock_node_protobufs::{
    com::hedera::hapi::block::stream::block_item::Item as BlockItemType,
    org::hiero::block::api::{
        publish_stream_request::Request as PublishRequestType,
        publish_stream_response, PublishStreamResponse,
    },
};
use std::sync::Arc;
use tokio::sync::mpsc;
use tonic::Status;
use tracing::{info, warn};
use uuid::Uuid;

/// Manages the state for a single, active publisher connection.
pub struct SessionManager {
    pub id: Uuid,
    context: AppContext,
    shared_state: Arc<SharedState>,
    state: SessionState,
    current_block_number: u64,
    item_buffer: Vec<rock_node_protobufs::com::hedera::hapi::block::stream::BlockItem>,
    response_tx: mpsc::Sender<std::result::Result<PublishStreamResponse, Status>>,
}

impl SessionManager {
    pub fn new(
        context: AppContext,
        shared_state: Arc<SharedState>,
        response_tx: mpsc::Sender<Result<PublishStreamResponse, Status>>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            context,
            shared_state,
            state: SessionState::New,
            current_block_number: 0,
            item_buffer: Vec::new(),
            response_tx,
        }
    }

    /// Processes a single request from the client stream. Returns `true` if the session should end.
    pub async fn handle_request(&mut self, request: PublishRequestType) -> bool {
        match request {
            PublishRequestType::BlockItems(block_item_set) => {
                for item in block_item_set.block_items {
                    if let Some(item_type) = &item.item {
                        if let BlockItemType::BlockHeader(header) = item_type {
                            if !self.handle_block_header(header.number as i64).await {
                                return true; // Handshake failed, end session.
                            }
                        }
                    }

                    if self.state == SessionState::Primary {
                        self.item_buffer.push(item.clone());
                    }

                    if let Some(BlockItemType::BlockProof(_)) = &item.item {
                        if self.state == SessionState::Primary {
                            self.publish_complete_block().await;
                            return true; // End session after successfully publishing a block.
                        }
                    }
                }
            }
            PublishRequestType::EndStream(_) => {
                info!(session_id = %self.id, "Publisher sent EndStream. Closing connection.");
                return true; // Exit the loop
            }
        }
        false
    }

    async fn handle_block_header(&mut self, block_number: i64) -> bool {
        self.current_block_number = block_number as u64;
        self.state = SessionState::New;
        self.item_buffer.clear();   

        let latest_persisted = self.shared_state.get_latest_persisted_block();
        if block_number <= latest_persisted {
            warn!(session_id = %self.id, block_number, latest_persisted, "Received duplicate block. Rejecting.");
            let code = publish_stream_response::end_of_stream::Code::DuplicateBlock;
            self.send_response(response_from_code(code, latest_persisted as u64)).await;
            return false;
        }

        let winner_entry = self.shared_state.block_winners.entry(block_number as u64).or_insert(self.id);
        if *winner_entry == self.id {
            self.state = SessionState::Primary;
            info!(session_id = %self.id, block_number, "This session is now PRIMARY.");
        } else {
            self.state = SessionState::Behind;
            info!(session_id = %self.id, block_number, "Another session is primary. Sending SkipBlock.");
            self.send_skip_block().await;
        }
        true
    }

    async fn publish_complete_block(&mut self) {
        info!(session_id = %self.id, block_number = self.current_block_number, "Block is complete. Publishing to core.");
        let block_data = rock_node_core::events::BlockData {
            block_number: self.current_block_number,
            contents: format!("Real data for block #{}. Contains {} items.", self.current_block_number, self.item_buffer.len()),
        };
        let cache_key = self.context.block_data_cache.insert(block_data);
        let event = rock_node_core::events::BlockItemsReceived {
            block_number: self.current_block_number,
            cache_key,
        };
        if self.context.tx_block_items_received.send(event).await.is_err() {
            warn!(session_id = %self.id, "Failed to publish BlockItemsReceived event.");
        }
        self.item_buffer.clear();
    }

    async fn send_skip_block(&self) {
        let response = PublishStreamResponse {
            response: Some(publish_stream_response::Response::SkipBlock(
                publish_stream_response::SkipBlock {
                    block_number: self.current_block_number,
                },
            )),
        };
        self.send_response(response).await;
    }

    async fn send_response(&self, response: PublishStreamResponse) {
        if self.response_tx.send(Ok(response)).await.is_err() {
            warn!(session_id = %self.id, "Failed to send response to client. Connection may be closed.");
        }
    }
}

fn response_from_code(code: publish_stream_response::end_of_stream::Code, block_number: u64) -> PublishStreamResponse {
    PublishStreamResponse {
        response: Some(publish_stream_response::Response::EndStream(
            publish_stream_response::EndOfStream {
                status: code.into(),
                block_number,
            },
        )),
    }
}
