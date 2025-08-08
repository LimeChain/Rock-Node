use crate::state::{SessionState, SharedState};
use anyhow::Result;
use prost::Message;
use rock_node_core::{
    app_context::AppContext,
    events::{BlockData, BlockItemsReceived},
};
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
use tokio::sync::{broadcast, mpsc};
use tonic::Status;
use tracing::{debug, error, info, trace, warn};
use uuid::Uuid;

pub struct ConsensusSession {
    pub id: Uuid,
    context: AppContext,
    shared_state: Arc<SharedState>,
    internal_tx: broadcast::Sender<BlockItemsReceived>,
    state: SessionState,
    current_block_number: u64,
    block_start_time: Option<Instant>,
    item_buffer: Vec<rock_node_protobufs::com::hedera::hapi::block::stream::BlockItem>,
    response_tx: mpsc::Sender<Result<PublishStreamResponse, Status>>,
    header_proof_total_duration: f64,
    header_proof_count: u64,
}

impl ConsensusSession {
    pub fn new(
        context: AppContext,
        shared_state: Arc<SharedState>,
        internal_tx: broadcast::Sender<BlockItemsReceived>,
        response_tx: mpsc::Sender<Result<PublishStreamResponse, Status>>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            context,
            shared_state,
            internal_tx,
            state: SessionState::New,
            current_block_number: 0,
            block_start_time: None,
            item_buffer: Vec::new(),
            response_tx,
            header_proof_total_duration: 0.0,
            header_proof_count: 0,
        }
    }

    pub async fn handle_request(&mut self, request: PublishRequestType) -> bool {
        match request {
            PublishRequestType::BlockItems(block_item_set) => {
                for item in block_item_set.block_items {
                    if let Some(item_type) = &item.item {
                        if let BlockItemType::BlockHeader(header) = item_type {
                            if !self.handle_block_header(header.number as i64).await {
                                return true;
                            }
                        }
                    }

                    if self.state == SessionState::Primary {
                        self.item_buffer.push(item.clone());
                        self.context.metrics.publish_items_processed_total.inc();
                    }

                    if let Some(BlockItemType::BlockProof(_)) = &item.item {
                        if self.state == SessionState::Primary {
                            if let Some(start) = self.block_start_time {
                                let duration = start.elapsed().as_secs_f64();
                                self.context
                                    .metrics
                                    .publish_header_to_proof_duration_seconds
                                    .with_label_values(&[])
                                    .observe(duration);
                                self.header_proof_total_duration += duration;
                                self.header_proof_count += 1;
                                let avg = self.header_proof_total_duration
                                    / self.header_proof_count as f64;
                                self.context
                                    .metrics
                                    .publish_average_header_to_proof_time_seconds
                                    .with_label_values(&[&self.id.to_string()])
                                    .set(avg);
                            }

                            info!(session_id = %self.id, "Received block proof. Publishing block.");
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
        self.block_start_time = None;
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
            self.block_start_time = Some(Instant::now());
            info!(session_id = %self.id, block_number, "Session is PRIMARY for this block.");
        } else {
            self.state = SessionState::Behind;
            info!(session_id = %self.id, block_number, "Another session is primary. Sending SkipBlock.");
            self.send_skip_block().await;
        }

        true
    }

    async fn publish_complete_block(&mut self) -> bool {
        info!(session_id = %self.id, block_number = self.current_block_number, "Block is complete. Publishing to internal channel.");

        let block_proto = Block {
            items: std::mem::take(&mut self.item_buffer),
        };

        let mut encoded_block_contents = Vec::new();
        if let Err(e) = block_proto.encode(&mut encoded_block_contents) {
            warn!(session_id = %self.id, error = %e, "Failed to encode complete block proto. Aborting publish.");
            return false;
        }

        let block_data = BlockData {
            block_number: self.current_block_number,
            contents: encoded_block_contents,
        };

        let cache_key = self.context.block_data_cache.insert(block_data);
        let event = BlockItemsReceived {
            block_number: self.current_block_number,
            cache_key,
        };

        if self.internal_tx.send(event).is_err() {
            warn!(session_id = %self.id, "Internal block channel has no receivers. Block will be dropped.");
        }

        if self.wait_for_persistence_ack().await.is_ok() {
            info!(session_id = %self.id, block_number = self.current_block_number, "Block persisted. Resetting session for next block.");
            self.reset_for_next_block();
            true
        } else {
            false
        }
    }

    async fn wait_for_persistence_ack(&mut self) -> Result<(), ()> {
        let mut rx_persisted = self.context.tx_block_persisted.subscribe();
        let block_to_await = self.current_block_number;
        let ack_timeout = Duration::from_secs(
            self.context
                .config
                .plugins
                .ingress_service
                .persistence_ack_timeout_seconds,
        );

        trace!(session_id = %self.id, block = block_to_await, "Awaiting persistence ACK...");

        let timeout_result = tokio::time::timeout(ack_timeout, async {
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
                info!(session_id = %self.id, block = block_to_await, "Block persisted. Broadcasting ACK and updating shared state.");
                self.shared_state
                    .set_latest_persisted_block(block_to_await as i64);

                let ack_msg = PublishStreamResponse {
                    response: Some(publish_stream_response::Response::Acknowledgement(
                        BlockAcknowledgement {
                            block_number: block_to_await,
                            block_root_hash: Vec::new(),
                        },
                    )),
                };

                for session_entry in self.shared_state.active_sessions.iter() {
                    let _ = session_entry.value().send(Ok(ack_msg.clone())).await;
                }
                self.shared_state.block_winners.remove(&block_to_await);
                Ok(())
            }
            _ => {
                warn!(session_id = %self.id, block = block_to_await, "Did not receive persistence ACK. Broadcasting ResendBlock request.");
                self.shared_state.block_winners.remove(&block_to_await);

                let resend_msg = PublishStreamResponse {
                    response: Some(publish_stream_response::Response::ResendBlock(
                        ResendBlock {
                            block_number: block_to_await,
                        },
                    )),
                };

                for session_entry in self.shared_state.active_sessions.iter() {
                    if *session_entry.key() != self.id {
                        let _ = session_entry.value().send(Ok(resend_msg.clone())).await;
                    }
                }
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
