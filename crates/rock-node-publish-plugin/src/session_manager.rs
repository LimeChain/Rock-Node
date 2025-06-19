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
                    }

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
                "Rejecting duplicate block based on startup state."
            );

            let code = publish_stream_response::end_of_stream::Code::DuplicateBlock;
            let response = response_from_code(code, latest_persisted as u64);
            self.send_response(response).await;

            return false;
        }

        if block_number > latest_persisted + 1 {
            warn!(
                session_id = %self.id,
                received_block = block_number,
                expected_block = latest_persisted + 1,
                "Rejecting future block. RockNode is BEHIND."
            );

            let code = publish_stream_response::end_of_stream::Code::Behind;
            let response = response_from_code(code, latest_persisted as u64);
            self.send_response(response).await;
            return false;
        }

        let winner_entry = self
            .shared_state
            .block_winners
            .entry(block_number as u64)
            .or_insert(self.id);
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

    async fn publish_complete_block(&mut self) {
        info!(session_id = %self.id, block_number = self.current_block_number, "Block is complete. Publishing to core.");

        // 1. Create the parent `Block` protobuf message.
        // `std::mem::take` efficiently moves the buffered items without needing to clone them.
        let block_proto = Block {
            items: std::mem::take(&mut self.item_buffer),
        };

        // 2. Encode the entire `Block` message into bytes.
        let mut encoded_block_contents = Vec::new();
        if let Err(e) = block_proto.encode(&mut encoded_block_contents) {
            warn!(session_id = %self.id, error = %e, "Failed to encode complete block proto. Aborting publish.");
            return; // Don't proceed if we can't encode the data
        }

        // 3. Create the event with the correctly encoded contents.
        let block_data = rock_node_core::events::BlockData {
            block_number: self.current_block_number,
            contents: encoded_block_contents, // Use the real data
        };

        // 4. Put the data in the cache and send the event to the persistence service.
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
            warn!(session_id = %self.id, "Failed to publish BlockItemsReceived event to core channel.");
            return;
        }

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
            })
            .await;

            match timeout_result {
                Ok(Some(_)) => {
                    info!(%session_id, block = block_to_await, "Block persisted. Sending session-specific ACK and updating shared state.");
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
                }
                _ => {
                    warn!(%session_id, block = block_to_await, "Did not receive persistence ACK for block. Requesting resend.");
                    let resend_req = PublishStreamResponse {
                        response: Some(publish_stream_response::Response::ResendBlock(
                            ResendBlock {
                                block_number: block_to_await,
                            },
                        )),
                    };
                    if response_tx_clone.send(Ok(resend_req)).await.is_err() {
                        warn!(%session_id, block = block_to_await, "Failed to send ResendBlock, client may have disconnected.");
                    }
                }
            }
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
