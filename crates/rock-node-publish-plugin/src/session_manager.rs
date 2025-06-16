use crate::state::{SharedState, SessionState};
use anyhow::Result;
use rock_node_core::AppContext;
use rock_node_protobufs::{
    com::hedera::hapi::block::stream::block_item::Item as BlockItemType,
    org::hiero::block::api::{
        publish_stream_request::Request as PublishRequestType,
        publish_stream_response::{self, BlockAcknowledgement, ResendBlock},
        PublishStreamResponse,
    },
};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tonic::Status;
use tracing::{info, warn};
use uuid::Uuid;

pub struct SessionManager {
    pub id: Uuid,
    context: AppContext,
    shared_state: Arc<SharedState>,
    state: SessionState,
    current_block_number: u64,
    item_buffer: Vec<rock_node_protobufs::com::hedera::hapi::block::stream::BlockItem>,
    response_tx: mpsc::Sender<Result<PublishStreamResponse, Status>>,
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

    pub async fn handle_request(&mut self, request: PublishRequestType) -> bool {
        match request {
            PublishRequestType::BlockItems(block_item_set) => {
                for item in block_item_set.block_items {
                    if let Some(item_type) = &item.item {
                        // The BlockHeader is the first item of any new block.
                        // This is where we perform the check.
                        if let BlockItemType::BlockHeader(header) = item_type {
                            // If `handle_block_header` returns false, it means the block
                            // was rejected and the stream should be terminated.
                            if !self.handle_block_header(header.number as i64).await {
                                return true; // Terminate connection.
                            }
                        }
                    }

                    // Only buffer items if this session is the primary one for this block.
                    if self.state == SessionState::Primary {
                        self.item_buffer.push(item.clone());
                    }

                    // The BlockProof is the last item. Once received, the block is complete.
                    if let Some(BlockItemType::BlockProof(_)) = &item.item {
                        if self.state == SessionState::Primary {
                            self.publish_complete_block().await;
                            self.reset_for_next_block();
                        }
                    }
                }
            }
            PublishRequestType::EndStream(_) => {
                info!(session_id = %self.id, "Publisher sent EndStream. Closing connection.");
                return true;
            }
        }
        false
    }

    /// Resets the session's buffer and state to be ready for the next block race.
    fn reset_for_next_block(&mut self) {
        info!(session_id = %self.id, block_number = self.current_block_number, "Processed block. Resetting session for next block.");
        self.item_buffer.clear();
        self.state = SessionState::New;
        self.current_block_number = 0;
    }
    
    /// Handles an incoming BlockHeader. It contains the logic to reject old blocks.
    async fn handle_block_header(&mut self, block_number: i64) -> bool {
        self.current_block_number = block_number as u64;
        self.state = SessionState::New;
        self.item_buffer.clear();

        // **CORRECTED BEHAVIOR START**
        // Fetch the latest block number that has been successfully persisted.
        // This state is initialized by the PublishPlugin at startup.
        let latest_persisted = self.shared_state.get_latest_persisted_block();

        // Per the design doc: "If this is less than last known verified block, respond with 'DuplicateBlock'".
        // Our check implements this. We use `<=` to handle blocks that are older or the same as the latest.
        if block_number <= latest_persisted {
            warn!(
                session_id = %self.id,
                received_block = block_number,
                latest_persisted_block = latest_persisted,
                "Rejecting duplicate block based on startup state."
            );
            
            // The protocol requires sending an EndOfStream message with a specific code.
            let code = publish_stream_response::end_of_stream::Code::DuplicateBlock;
            
            // The protocol also states: "Response includes the last known block".
            let response = response_from_code(code, latest_persisted as u64);
            self.send_response(response).await;
            
            // Returning `false` signals the calling handler to terminate this session's stream.
            return false;
        }
        // **CORRECTED BEHAVIOR END**

        // If the block is not a duplicate, proceed with the multi-publisher leader election.
        let winner_entry = self.shared_state.block_winners.entry(block_number as u64).or_insert(self.id);
        if *winner_entry == self.id {
            self.state = SessionState::Primary;
            info!(session_id = %self.id, block_number, "Session is PRIMARY for this block.");
        } else {
            self.state = SessionState::Behind;
            info!(session_id = %self.id, block_number, "Another session is primary. Sending SkipBlock.");
            self.send_skip_block().await;
        }
        
        // Returning `true` signals that the session should continue.
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
            return;
        }
        self.item_buffer.clear();

        self.wait_for_persistence_ack().await;
    }

    async fn wait_for_persistence_ack(&self) {
        let mut rx_persisted = self.context.tx_block_persisted.subscribe();
        let response_tx_clone = self.response_tx.clone();
        let block_to_await = self.current_block_number;
        let session_id = self.id;
        let shared_state_clone = self.shared_state.clone();

        tokio::spawn(async move {
            info!(%session_id, block = block_to_await, "Spawned ACK waiter task.");
            
            let timeout_result = tokio::time::timeout(Duration::from_secs(30), async {
                while let Ok(persisted_event) = rx_persisted.recv().await {
                    if persisted_event.block_number == block_to_await {
                        return Some(persisted_event);
                    }
                }
                None
            }).await;

            match timeout_result {
                Ok(Some(_)) => {
                    info!(%session_id, block = block_to_await, "Block persisted. Sending session-specific ACK and updating shared state.");
                    // IMPORTANT: Update the shared state so future connections know about this newly persisted block.
                    shared_state_clone.set_latest_persisted_block(block_to_await as i64);
                    let ack = PublishStreamResponse {
                        response: Some(publish_stream_response::Response::Acknowledgement(
                            BlockAcknowledgement { 
                                block_number: block_to_await,
                                block_already_exists: false,
                                block_root_hash: vec![],
                             },
                        )),
                    };
                    if response_tx_clone.send(Ok(ack)).await.is_err() {
                        warn!(%session_id, block = block_to_await, "Failed to send ACK, client may have disconnected.");
                    }
                },
                _ => {
                    warn!(%session_id, block = block_to_await, "Did not receive persistence ACK for block. Requesting resend.");
                    let resend_req = PublishStreamResponse {
                        response: Some(publish_stream_response::Response::ResendBlock(
                            ResendBlock { block_number: block_to_await },
                        )),
                    };
                    if response_tx_clone.send(Ok(resend_req)).await.is_err() {
                         warn!(%session_id, block = block_to_await, "Failed to send ResendBlock, client may have disconnected.");
                    }
                }
            }
            // After processing, remove this block's winner to allow garbage collection.
            shared_state_clone.block_winners.remove(&block_to_await);
        });
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
