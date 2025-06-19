use crate::state::{SessionState, SharedState};
use anyhow::Result;
use prost::Message;
use rock_node_core::AppContext;
use rock_node_protobufs::{
    com::hedera::hapi::block::stream::block_item::Item as BlockItemType,
    com::hedera::hapi::block::stream::Block,
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

    /// Handles a request from the client stream.
    /// Returns `true` if the stream should be terminated, `false` otherwise.
    pub async fn handle_request(&mut self, request: PublishRequestType) -> bool {
        match request {
            PublishRequestType::BlockItems(block_item_set) => {
                for item in block_item_set.block_items {
                    if let Some(item_type) = &item.item {
                        if let BlockItemType::BlockHeader(header) = item_type {
                            if !self.handle_block_header(header.number as i64).await {
                                return true; // Terminate stream due to header error
                            }
                        }
                    }

                    if self.state == SessionState::Primary {
                        self.item_buffer.push(item.clone());
                    }

                    if let Some(BlockItemType::BlockProof(_)) = &item.item {
                        if self.state == SessionState::Primary {
                            // Await the full publish-and-persist cycle.
                            // If it fails, terminate the stream.
                            if !self.publish_complete_block().await {
                                return true;
                            }
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

    fn reset_for_next_block(&mut self) {
        info!(session_id = %self.id, block_number = self.current_block_number, "Processed block. Resetting session for next block.");
        self.item_buffer.clear();
        self.state = SessionState::New;
        self.current_block_number = 0;
    }

    async fn handle_block_header(&mut self, block_number: i64) -> bool {
        self.current_block_number = block_number as u64;
        self.state = SessionState::New;
        self.item_buffer.clear();

        let latest_persisted = self.shared_state.get_latest_persisted_block();

        if block_number <= latest_persisted {
            warn!(
                session_id = %self.id,
                received_block = block_number,
                latest_persisted_block = latest_persisted,
                "Rejecting duplicate block."
            );
            let response = response_from_code(publish_stream_response::end_of_stream::Code::DuplicateBlock, latest_persisted as u64);
            let _ = self.send_response(response).await;
            return false;
        }

        if block_number > latest_persisted + 1 {
            warn!(
                session_id = %self.id,
                received_block = block_number,
                expected_block = latest_persisted + 1,
                "Rejecting future block. RockNode is BEHIND."
            );
            let response = response_from_code(publish_stream_response::end_of_stream::Code::Behind, latest_persisted as u64);
            let _ = self.send_response(response).await;
            return false;
        }

        let winner_entry = self.shared_state.block_winners.entry(block_number as u64).or_insert(self.id);
        if *winner_entry == self.id {
            self.state = SessionState::Primary;
            info!(session_id = %self.id, block_number, "Session is PRIMARY for this block.");
        } else {
            self.state = SessionState::Behind;
            info!(session_id = %self.id, block_number, "Another session is primary. Sending SkipBlock.");
            self.send_skip_block().await;
        }

        true
    }

    /// Returns `true` on success, `false` on failure.
    async fn publish_complete_block(&mut self) -> bool {
        info!(session_id = %self.id, block_number = self.current_block_number, "Block is complete. Publishing to core.");

        let block_proto = Block {
            items: std::mem::take(&mut self.item_buffer),
        };

        let mut encoded_block_contents = Vec::new();
        if let Err(e) = block_proto.encode(&mut encoded_block_contents) {
            warn!(session_id = %self.id, error = %e, "Failed to encode complete block proto. Aborting publish.");
            return false;
        }

        let block_data = rock_node_core::events::BlockData {
            block_number: self.current_block_number,
            contents: encoded_block_contents,
        };

        let cache_key = self.context.block_data_cache.insert(block_data);
        let event = rock_node_core::events::BlockItemsReceived {
            block_number: self.current_block_number,
            cache_key,
        };

        if self.context.tx_block_items_received.send(event).await.is_err() {
            warn!(session_id = %self.id, "Failed to publish BlockItemsReceived event to core channel.");
            return false;
        }

        // AWAIT the persistence result. This is the critical change for flow control.
        if self.wait_for_persistence_ack().await.is_ok() {
            self.reset_for_next_block();
            true
        } else {
            // ACK failed or timed out, signal failure to the main loop.
            false
        }
    }

    /// Waits for the persistence event. Does NOT spawn a separate task.
    /// Returns Ok on success, Err on timeout or channel close.
    async fn wait_for_persistence_ack(&mut self) -> Result<(), ()> {
        let mut rx_persisted = self.context.tx_block_persisted.subscribe();
        let block_to_await = self.current_block_number;

        info!(session_id = %self.id, block = block_to_await, "Awaiting persistence ACK...");

        let timeout_result = tokio::time::timeout(Duration::from_secs(30), async {
            while let Ok(persisted_event) = rx_persisted.recv().await {
                if persisted_event.block_number == block_to_await {
                    return Some(persisted_event);
                }
            }
            None
        }).await;

        // Clean up the winner map regardless of the outcome.
        self.shared_state.block_winners.remove(&block_to_await);

        match timeout_result {
            Ok(Some(_)) => {
                info!(session_id = %self.id, block = block_to_await, "Block persisted. Sending ACK and updating shared state.");
                self.shared_state.set_latest_persisted_block(block_to_await as i64);

                let ack = PublishStreamResponse {
                    response: Some(publish_stream_response::Response::Acknowledgement(
                        BlockAcknowledgement {
                            block_number: block_to_await,
                            block_already_exists: false,
                            block_root_hash: Vec::new(),
                        },
                    )),
                };
                if self.send_response(ack).await.is_err() {
                     // Client likely disconnected, which is an error state for this cycle.
                    return Err(());
                }
                Ok(())
            }
            _ => {
                warn!(session_id = %self.id, block = block_to_await, "Did not receive persistence ACK for block. Requesting resend.");
                let resend_req = PublishStreamResponse {
                    response: Some(publish_stream_response::Response::ResendBlock(
                        ResendBlock { block_number: block_to_await },
                    )),
                };
                let _ = self.send_response(resend_req).await;
                Err(())
            }
        }
    }

    async fn send_skip_block(&self) {
        let response = PublishStreamResponse {
            response: Some(publish_stream_response::Response::SkipBlock(
                publish_stream_response::SkipBlock {
                    block_number: self.current_block_number,
                },
            )),
        };
        let _ = self.send_response(response).await;
    }

    async fn send_response(&self, response: PublishStreamResponse) -> Result<(),()> {
        if self.response_tx.send(Ok(response)).await.is_err() {
            warn!(session_id = %self.id, "Failed to send response to client. Connection may be closed.");
            return Err(());
        }
        Ok(())
    }
}

fn response_from_code(
    code: publish_stream_response::end_of_stream::Code,
    block_number: u64,
) -> PublishStreamResponse {
    PublishStreamResponse {
        response: Some(publish_stream_response::Response::EndStream(
            publish_stream_response::EndOfStream {
                status: code.into(),
                block_number,
            },
        )),
    }
}
