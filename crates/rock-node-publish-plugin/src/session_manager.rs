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
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tonic::Status;
use tracing::{debug, error, info, trace, warn};
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
                        self.context.metrics.publish_items_processed_total.inc();
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
                debug!(session_id = %self.id, "Publisher sent EndStream. Closing connection.");
                return true;
            }
        }
        false
    }

    fn reset_for_next_block(&mut self) {
        debug!(session_id = %self.id, block_number = self.current_block_number, "Processed block. Resetting session for next block.");
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
            info!(
                session_id = %self.id,
                received_block = block_number,
                latest_persisted_block = latest_persisted,
                "Rejecting duplicate block."
            );
            self.context
                .metrics
                .publish_blocks_received_total
                .with_label_values(&["duplicate"])
                .inc();
            self.context
                .metrics
                .publish_responses_sent_total
                .with_label_values(&["EndStream_DuplicateBlock"])
                .inc();
            let response = response_from_code(
                publish_stream_response::end_of_stream::Code::DuplicateBlock,
                latest_persisted as u64,
            );
            let _ = self.send_response(response).await;
            return false;
        }

        if block_number > latest_persisted + 1 {
            info!(
                session_id = %self.id,
                received_block = block_number,
                expected_block = latest_persisted + 1,
                "Rejecting future block. RockNode is BEHIND."
            );
            self.context
                .metrics
                .publish_blocks_received_total
                .with_label_values(&["future_block"])
                .inc();
            self.context
                .metrics
                .publish_responses_sent_total
                .with_label_values(&["EndStream_Behind"])
                .inc();
            let response = response_from_code(
                publish_stream_response::end_of_stream::Code::Behind,
                latest_persisted as u64,
            );
            let _ = self.send_response(response).await;
            return false;
        }

        let winner_entry = self
            .shared_state
            .block_winners
            .entry(block_number as u64)
            .or_insert(self.id);
        if *winner_entry == self.id {
            self.state = SessionState::Primary;
            self.context
                .metrics
                .publish_blocks_received_total
                .with_label_values(&["primary"])
                .inc();
            info!(session_id = %self.id, block_number, "Session is PRIMARY for this block.");
        } else {
            self.state = SessionState::Behind;
            self.context
                .metrics
                .publish_blocks_received_total
                .with_label_values(&["behind"])
                .inc();
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

        if self
            .context
            .tx_block_items_received
            .send(event)
            .await
            .is_err()
        {
            error!(session_id = %self.id, "Failed to publish BlockItemsReceived event to core channel.");
            return false;
        }

        if self.wait_for_persistence_ack().await.is_ok() {
            self.reset_for_next_block();
            true
        } else {
            // ACK failed or timed out. The logic inside wait_for_persistence_ack handles retries.
            // Terminate this session's attempt.
            false
        }
    }

    /// Waits for the persistence event and broadcasts the result to all active sessions.
    async fn wait_for_persistence_ack(&mut self) -> Result<(), ()> {
        let mut rx_persisted = self.context.tx_block_persisted.subscribe();
        let block_to_await = self.current_block_number;

        trace!(session_id = %self.id, block = block_to_await, "Awaiting persistence ACK...");
        let start_time = Instant::now();

        // This is only ever called by the PRIMARY session.
        let timeout_result = tokio::time::timeout(Duration::from_secs(30), async {
            while let Ok(persisted_event) = rx_persisted.recv().await {
                if persisted_event.block_number == block_to_await {
                    return Some(persisted_event);
                }
            }
            None
        })
        .await;

        match timeout_result {
            Ok(Some(_)) => {
                // SUCCESS CASE
                info!(session_id = %self.id, block = block_to_await, "Block persisted. Broadcasting ACK and updating shared state.");

                let duration = start_time.elapsed().as_secs_f64();
                self.context
                    .metrics
                    .publish_persistence_duration_seconds
                    .with_label_values(&["acknowledged"])
                    .observe(duration);
                self.context.metrics.blocks_acknowledged.inc();

                // Update shared state *before* broadcasting
                self.shared_state
                    .set_latest_persisted_block(block_to_await as i64);

                // Create the acknowledgement message
                let ack_msg = PublishStreamResponse {
                    response: Some(publish_stream_response::Response::Acknowledgement(
                        BlockAcknowledgement {
                            block_number: block_to_await,
                            block_already_exists: false,
                            block_root_hash: Vec::new(),
                        },
                    )),
                };

                // Broadcast to all active sessions
                for session_entry in self.shared_state.active_sessions.iter() {
                    let _ = session_entry.value().send(Ok(ack_msg.clone())).await;
                }

                self.context
                    .metrics
                    .publish_responses_sent_total
                    .with_label_values(&["Acknowledgement"])
                    .inc();

                // Clean up the winner map for this block
                self.shared_state.block_winners.remove(&block_to_await);
                Ok(())
            }
            _ => {
                // FAILURE/TIMEOUT CASE
                warn!(session_id = %self.id, block = block_to_await, "Did not receive persistence ACK. Broadcasting ResendBlock request.");

                let duration = start_time.elapsed().as_secs_f64();
                self.context
                    .metrics
                    .publish_persistence_duration_seconds
                    .with_label_values(&["timeout"])
                    .observe(duration);

                // IMPORTANT: Remove the failed winner to allow a new race
                self.shared_state.block_winners.remove(&block_to_await);

                // Create the ResendBlock message
                let resend_msg = PublishStreamResponse {
                    response: Some(publish_stream_response::Response::ResendBlock(
                        ResendBlock {
                            block_number: block_to_await,
                        },
                    )),
                };

                // Broadcast to all other sessions to trigger a new race for this block
                for session_entry in self.shared_state.active_sessions.iter() {
                    if *session_entry.key() != self.id {
                        // Don't ask the failed session to resend
                        let _ = session_entry.value().send(Ok(resend_msg.clone())).await;
                    }
                }

                self.context
                    .metrics
                    .publish_responses_sent_total
                    .with_label_values(&["ResendBlock"])
                    .inc();

                // Terminate this session's attempt.
                Err(())
            }
        }
    }

    async fn send_skip_block(&self) {
        self.context
            .metrics
            .publish_responses_sent_total
            .with_label_values(&["SkipBlock"])
            .inc();
        let response = PublishStreamResponse {
            response: Some(publish_stream_response::Response::SkipBlock(
                publish_stream_response::SkipBlock {
                    block_number: self.current_block_number,
                },
            )),
        };
        let _ = self.send_response(response).await;
    }

    async fn send_response(&self, response: PublishStreamResponse) -> Result<(), ()> {
        if self.response_tx.send(Ok(response)).await.is_err() {
            debug!(session_id = %self.id, "Failed to send response to client. Connection may be closed.");
            Err(())
        } else {
            Ok(())
        }
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
