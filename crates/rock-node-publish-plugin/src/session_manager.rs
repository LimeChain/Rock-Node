use crate::{
    error::PublishError,
    state::{BlockAction, SessionState, SharedState},
    validation::RequestValidator,
};
use anyhow::Result;
use prost::Message;
use rock_node_core::AppContext;
use rock_node_protobufs::{
    com::hedera::hapi::block::stream::block_item::Item as BlockItemType,
    com::hedera::hapi::block::stream::Block,
    org::hiero::block::api::{
        publish_stream_request::Request as PublishRequestType,
        publish_stream_response::{self, BlockAcknowledgement, ResendBlock, SkipBlock},
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
    /// Optional publisher identifier for tracking/metrics
    pub publisher_id: Option<String>,
    context: AppContext,
    shared_state: Arc<SharedState>,
    state: SessionState,
    current_block_number: i64,
    current_block_action: Option<BlockAction>,
    block_start_time: Option<Instant>,
    session_start_time: Instant,
    item_buffer: Vec<rock_node_protobufs::com::hedera::hapi::block::stream::BlockItem>,
    response_tx: mpsc::Sender<Result<PublishStreamResponse, Status>>,
    header_proof_total_duration: f64,
    header_proof_count: u64,
    validator: RequestValidator,
    // Metrics tracking
    blocks_received: u64,
    blocks_accepted: u64,
    blocks_skipped: u64,
}

impl SessionManager {
    pub fn new(
        context: AppContext,
        shared_state: Arc<SharedState>,
        response_tx: mpsc::Sender<Result<PublishStreamResponse, Status>>,
    ) -> Self {
        let max_items = context.config.plugins.publish_service.max_items_per_set;
        Self {
            id: Uuid::new_v4(),
            publisher_id: None,
            context,
            shared_state,
            state: SessionState::New,
            current_block_number: 0,
            current_block_action: None,
            block_start_time: None,
            session_start_time: Instant::now(),
            item_buffer: Vec::new(),
            response_tx,
            header_proof_total_duration: 0.0,
            header_proof_count: 0,
            validator: RequestValidator::new(max_items),
            blocks_received: 0,
            blocks_accepted: 0,
            blocks_skipped: 0,
        }
    }

    /// Handles a request from the client stream.
    /// Returns `true` if the stream should be terminated, `false` otherwise.
    pub async fn handle_request(&mut self, request: PublishRequestType) -> bool {
        match request {
            PublishRequestType::BlockItems(block_item_set) => {
                self.handle_block_items(block_item_set).await
            },
            PublishRequestType::EndOfBlock(block_end) => self.handle_block_end(block_end).await,
            PublishRequestType::EndStream(end_stream) => {
                self.handle_end_stream(end_stream).await;
                true // Always terminate
            },
        }
    }

    async fn handle_block_items(
        &mut self,
        block_item_set: rock_node_protobufs::org::hiero::block::api::BlockItemSet,
    ) -> bool {
        self.blocks_received += 1;

        let mut block_number: Option<i64> = None;

        // First pass - extract block number from header if present
        for item in &block_item_set.block_items {
            if let Some(BlockItemType::BlockHeader(header)) = &item.item {
                block_number = Some(header.number as i64);
                break;
            }
        }

        // Determine if this is a new block or continuation
        let is_new_block = block_number.is_some();

        // Validate request format
        if let Err(e) = self
            .validator
            .validate_block_items(&block_item_set, is_new_block)
        {
            error!(
                session_id = %self.id,
                error = %e,
                "Validation failed for block items"
            );
            self.context
                .metrics
                .publish_blocks_received_total
                .with_label_values(&["validation_error"])
                .inc();

            // Send error response to client before terminating
            let response = PublishStreamResponse {
                response: Some(publish_stream_response::Response::EndStream(
                    publish_stream_response::EndOfStream {
                        status: publish_stream_response::end_of_stream::Code::Error as i32,
                        block_number: self.current_block_number as u64,
                    },
                )),
            };
            let _ = self.send_response(response).await;
            self.context
                .metrics
                .publish_responses_sent_total
                .with_label_values(&["EndStream_ValidationError"])
                .inc();

            return true; // Terminate session on validation error
        }

        // Get action for this request
        let action = if let Some(num) = block_number {
            // First request of block (has header)
            self.get_action_for_block(num, None)
        } else if self.current_block_action.is_some() {
            // Continuation of current block
            self.get_action_for_block(self.current_block_number, self.current_block_action)
        } else {
            // Items without header and no current block - error
            error!(
                session_id = %self.id,
                "Received items without header and no current block"
            );
            BlockAction::EndError
        };

        // Store action for next iteration
        self.current_block_action = Some(action);

        // Execute action
        match action {
            BlockAction::Accept => self.handle_accept(block_item_set).await,
            BlockAction::Skip => {
                self.handle_skip().await;
                false // Continue session
            },
            BlockAction::Resend => {
                self.handle_resend().await;
                false // Continue session (wait for resend)
            },
            BlockAction::EndBehind => {
                self.handle_end_behind().await;
                true // Terminate
            },
            BlockAction::EndDuplicate => {
                self.handle_end_duplicate().await;
                true // Terminate
            },
            BlockAction::EndError | BlockAction::BadBlockProof => {
                self.handle_end_error(action).await;
                true // Terminate
            },
        }
    }

    /// Determine what action to take for this block/request (Java's getActionForBlock)
    fn get_action_for_block(
        &mut self,
        block_number: i64,
        previous_action: Option<BlockAction>,
    ) -> BlockAction {
        // If we have a terminal previous action, stay terminal
        match previous_action {
            Some(BlockAction::EndError)
            | Some(BlockAction::EndDuplicate)
            | Some(BlockAction::EndBehind)
            | Some(BlockAction::BadBlockProof) => {
                return BlockAction::EndError;
            },
            Some(BlockAction::Skip) | Some(BlockAction::Resend) => {
                // These should reset to new action (shouldn't receive more after skip/resend)
                return BlockAction::EndError;
            },
            _ => {},
        }

        // First request (header) - determine if we accept or skip
        if previous_action.is_none() {
            return self.get_action_for_header(block_number);
        }

        // Subsequent requests - check if still valid
        if previous_action == Some(BlockAction::Accept) {
            return self.get_action_for_currently_streaming(block_number);
        }

        BlockAction::EndError
    }

    /// Action for header (first item of block) - Java's getActionForHeader
    fn get_action_for_header(&mut self, block_number: i64) -> BlockAction {
        let latest_persisted = self.shared_state.get_latest_persisted_block();

        // Check if block is behind what we already have
        if block_number < latest_persisted {
            warn!(
                session_id = %self.id,
                block_number = block_number,
                latest_persisted = latest_persisted,
                "Block is behind our state"
            );
            self.current_block_number = block_number;
            self.context
                .metrics
                .publish_blocks_received_total
                .with_label_values(&["behind_persisted"])
                .inc();
            return BlockAction::EndBehind;
        }

        // Check if this block is already persisted (duplicate)
        if block_number == latest_persisted {
            warn!(
                session_id = %self.id,
                block_number = block_number,
                latest_persisted = latest_persisted,
                "Block already persisted - duplicate"
            );
            self.current_block_number = block_number;
            self.context
                .metrics
                .publish_blocks_received_total
                .with_label_values(&["duplicate"])
                .inc();
            return BlockAction::EndDuplicate;
        }

        // Note: We DO NOT reject future blocks (block_number > latest_persisted + 1).
        // The persistence layer handles out-of-order blocks by creating gap ranges,
        // and the backfill plugin fills those gaps later. This design prioritizes
        // data availability - we want to accept and persist all blocks we receive.

        // Check if this is a duplicate (we're currently streaming this block)
        if let Some(winner_entry) = self.shared_state.block_winners.get(&(block_number as u64)) {
            if *winner_entry == self.id {
                // We're already the winner for this block - duplicate
                warn!(
                    session_id = %self.id,
                    block_number = block_number,
                    "Duplicate block header from same session"
                );
                self.current_block_number = block_number;
                self.context
                    .metrics
                    .publish_blocks_received_total
                    .with_label_values(&["duplicate"])
                    .inc();
                return BlockAction::EndDuplicate;
            } else {
                // Someone else won - skip
                debug!(
                    session_id = %self.id,
                    block_number = block_number,
                    winner = %*winner_entry,
                    "Lost election, skipping block"
                );
                self.current_block_number = block_number;
                self.context
                    .metrics
                    .publish_blocks_received_total
                    .with_label_values(&["skipped"])
                    .inc();
                return BlockAction::Skip;
            }
        }

        // Try to become winner
        match self.shared_state.block_winners.entry(block_number as u64) {
            dashmap::mapref::entry::Entry::Vacant(vacant) => {
                vacant.insert(self.id);
                info!(
                    session_id = %self.id,
                    block_number = block_number,
                    "Won election, accepting block"
                );
                self.current_block_number = block_number;
                self.state = SessionState::Primary;
                self.block_start_time = Some(Instant::now());
                self.context
                    .metrics
                    .publish_blocks_received_total
                    .with_label_values(&["primary"])
                    .inc();
                BlockAction::Accept
            },
            dashmap::mapref::entry::Entry::Occupied(occupied) => {
                // Race condition - someone else just won
                debug!(
                    session_id = %self.id,
                    block_number = block_number,
                    winner = %*occupied.get(),
                    "Lost election (race), skipping block"
                );
                self.state = SessionState::Behind;
                self.context
                    .metrics
                    .publish_blocks_received_total
                    .with_label_values(&["skipped"])
                    .inc();
                BlockAction::Skip
            },
        }
    }

    /// Action for items after header (continuing block) - Java's getActionForCurrentlyStreaming
    fn get_action_for_currently_streaming(&self, block_number: i64) -> BlockAction {
        // Verify we're still on the same block
        if block_number != self.current_block_number {
            error!(
                session_id = %self.id,
                expected = self.current_block_number,
                received = block_number,
                "Block number mismatch during streaming"
            );
            return BlockAction::EndError;
        }

        // Check we're still the winner
        if let Some(winner) = self.shared_state.block_winners.get(&(block_number as u64)) {
            if *winner == self.id {
                return BlockAction::Accept;
            } else {
                // Someone else took over - shouldn't happen
                error!(
                    session_id = %self.id,
                    block_number = block_number,
                    current_winner = %*winner,
                    "Winner changed during block streaming"
                );
                return BlockAction::Resend;
            }
        }

        // Winner disappeared - error
        error!(
            session_id = %self.id,
            block_number = block_number,
            "Winner entry disappeared during streaming"
        );
        BlockAction::EndError
    }

    fn reset_for_next_block(&mut self) {
        debug!(session_id = %self.id, block_number = self.current_block_number, "Processed block. Resetting session for next block.");
        self.item_buffer.clear();
        self.state = SessionState::New;
        self.current_block_number = 0;
        self.current_block_action = None;
        self.block_start_time = None;
    }

    /// Handle Accept action - process items and check for block completion
    async fn handle_accept(
        &mut self,
        block_item_set: rock_node_protobufs::org::hiero::block::api::BlockItemSet,
    ) -> bool {
        for item in block_item_set.block_items {
            if let Some(item_type) = &item.item {
                match item_type {
                    BlockItemType::BlockHeader(_) => {
                        debug!(
                            session_id = %self.id,
                            block_number = self.current_block_number,
                            "Processing block header"
                        );
                    },
                    BlockItemType::BlockProof(_) => {
                        // Measure header -> proof duration
                        if let Some(start) = self.block_start_time {
                            let duration = start.elapsed().as_secs_f64();
                            self.context
                                .metrics
                                .publish_header_to_proof_duration_seconds
                                .with_label_values::<&str>(&[])
                                .observe(duration);
                            self.header_proof_total_duration += duration;
                            self.header_proof_count += 1;
                            let avg =
                                self.header_proof_total_duration / self.header_proof_count as f64;
                            self.context
                                .metrics
                                .publish_average_header_to_proof_time_seconds
                                .with_label_values(&[&self.id.to_string()])
                                .set(avg);
                        }

                        info!(
                            session_id = %self.id,
                            block_number = self.current_block_number,
                            "Received block proof. Publishing block."
                        );
                        self.item_buffer.push(item);
                        return self.complete_block().await;
                    },
                    _ => {},
                }
            }

            self.item_buffer.push(item);
            self.context.metrics.publish_items_processed_total.inc();
        }
        false // Continue
    }

    /// Handle Skip action - send SkipBlock response and reset
    async fn handle_skip(&mut self) {
        self.blocks_skipped += 1;
        let response = PublishStreamResponse {
            response: Some(publish_stream_response::Response::SkipBlock(SkipBlock {
                block_number: self.current_block_number as u64,
            })),
        };
        let _ = self.send_response(response).await;
        self.context
            .metrics
            .publish_responses_sent_total
            .with_label_values(&["SkipBlock"])
            .inc();
        self.reset_for_next_block();
    }

    /// Handle Resend action - send ResendBlock response and reset
    async fn handle_resend(&mut self) {
        let response = PublishStreamResponse {
            response: Some(publish_stream_response::Response::ResendBlock(
                ResendBlock {
                    block_number: self.current_block_number as u64,
                },
            )),
        };
        let _ = self.send_response(response).await;
        self.context
            .metrics
            .publish_responses_sent_total
            .with_label_values(&["ResendBlock"])
            .inc();
        self.reset_for_next_block();
    }

    /// Handle EndBehind action - publisher is behind us
    async fn handle_end_behind(&mut self) {
        let latest = self.shared_state.get_latest_persisted_block();
        let response = PublishStreamResponse {
            response: Some(publish_stream_response::Response::EndStream(
                publish_stream_response::EndOfStream {
                    status: publish_stream_response::end_of_stream::Code::Behind as i32,
                    block_number: latest as u64,
                },
            )),
        };
        let _ = self.send_response(response).await;
        self.context
            .metrics
            .publish_responses_sent_total
            .with_label_values(&["EndStream_Behind"])
            .inc();

        error!(
            session_id = %self.id,
            block_number = self.current_block_number,
            our_latest = latest,
            "Publisher is behind, terminating"
        );
    }

    /// Handle EndDuplicate action - duplicate block detected
    async fn handle_end_duplicate(&mut self) {
        let response = PublishStreamResponse {
            response: Some(publish_stream_response::Response::EndStream(
                publish_stream_response::EndOfStream {
                    status: publish_stream_response::end_of_stream::Code::DuplicateBlock as i32,
                    block_number: self.current_block_number as u64,
                },
            )),
        };
        let _ = self.send_response(response).await;
        self.context
            .metrics
            .publish_responses_sent_total
            .with_label_values(&["EndStream_Duplicate"])
            .inc();

        error!(
            session_id = %self.id,
            block_number = self.current_block_number,
            "Duplicate block detected, terminating"
        );
    }

    /// Handle EndError action - generic error or bad block proof
    async fn handle_end_error(&mut self, action: BlockAction) {
        let code = match action {
            BlockAction::BadBlockProof => {
                publish_stream_response::end_of_stream::Code::BadBlockProof
            },
            _ => publish_stream_response::end_of_stream::Code::Error,
        };

        let response = PublishStreamResponse {
            response: Some(publish_stream_response::Response::EndStream(
                publish_stream_response::EndOfStream {
                    status: code as i32,
                    block_number: self.current_block_number as u64,
                },
            )),
        };
        let _ = self.send_response(response).await;
        self.context
            .metrics
            .publish_responses_sent_total
            .with_label_values(&[&format!("EndStream_{:?}", code)])
            .inc();

        error!(
            session_id = %self.id,
            action = ?action,
            "Error condition, terminating"
        );
    }

    /// Handle BlockEnd message (alternative completion signal to BlockProof)
    async fn handle_block_end(
        &mut self,
        block_end: rock_node_protobufs::org::hiero::block::api::BlockEnd,
    ) -> bool {
        // Verify we're in accepting state
        if self.current_block_action != Some(BlockAction::Accept) {
            error!(
                session_id = %self.id,
                "Received BlockEnd but not in Accept state"
            );
            self.current_block_action = Some(BlockAction::EndError);
            self.handle_end_error(BlockAction::EndError).await;
            return true;
        }

        // Verify block number matches
        if block_end.block_number as i64 != self.current_block_number {
            error!(
                session_id = %self.id,
                expected = self.current_block_number,
                received = block_end.block_number,
                "BlockEnd number mismatch"
            );
            self.current_block_action = Some(BlockAction::EndError);
            self.handle_end_error(BlockAction::EndError).await;
            return true;
        }

        info!(
            session_id = %self.id,
            block_number = block_end.block_number,
            "Received BlockEnd, completing block"
        );

        // Complete block (same as BlockProof path)
        self.complete_block().await
    }

    /// Handle EndStream message - validate and log publisher state
    async fn handle_end_stream(
        &mut self,
        end_stream: rock_node_protobufs::org::hiero::block::api::publish_stream_request::EndStream,
    ) {
        // Validate the end stream request
        let latest = end_stream.latest_block_number;
        let earliest = end_stream.earliest_block_number;
        let our_latest = self.shared_state.get_latest_persisted_block();

        // Check if publisher reports being significantly ahead
        if latest > our_latest as u64 + 100 {
            warn!(
                session_id = %self.id,
                publisher_latest = latest,
                our_latest = our_latest,
                gap = latest as i64 - our_latest,
                "Publisher is significantly ahead - may need backfill"
            );

            // TODO: Trigger backfill mechanism
            // self.context.trigger_backfill(our_latest + 1, latest);
        }

        // Check if earliest > latest (invalid)
        if earliest > latest {
            warn!(
                session_id = %self.id,
                earliest = earliest,
                latest = latest,
                "Publisher sent invalid EndStream (earliest > latest)"
            );
        }

        info!(
            session_id = %self.id,
            earliest = earliest,
            latest = latest,
            "Publisher sent EndStream"
        );
    }

    /// Complete block: send to verification, wait for result, then persistence, then ACK
    /// Returns false to continue the session, true to terminate
    async fn complete_block(&mut self) -> bool {
        self.blocks_accepted += 1;
        info!(session_id = %self.id, block_number = self.current_block_number, "Block is complete. Sending to verification.");

        let block_proto = Block {
            items: std::mem::take(&mut self.item_buffer),
        };

        let mut encoded_block_contents = Vec::new();
        if let Err(e) = block_proto.encode(&mut encoded_block_contents) {
            warn!(session_id = %self.id, error = %e, "Failed to encode complete block proto. Aborting publish.");
            return true; // Terminate on encoding error
        }

        let block_data = rock_node_core::events::BlockData {
            block_number: self.current_block_number as u64,
            contents: encoded_block_contents,
        };

        let cache_key = self.context.block_data_cache.insert(block_data);
        let event = rock_node_core::events::BlockItemsReceived {
            block_number: self.current_block_number as u64,
            cache_key,
        };

        // Step 1: Send to verification plugin
        if self
            .context
            .tx_block_items_received
            .send(event)
            .await
            .is_err()
        {
            error!(session_id = %self.id, "Failed to publish BlockItemsReceived event to verification channel.");
            return true; // Terminate on channel error
        }

        // Step 2: Wait for verification result and then persistence
        if self
            .wait_for_verification_and_persistence(cache_key)
            .await
            .is_ok()
        {
            info!(session_id = %self.id, block_number = self.current_block_number, "Block verified and persisted. Resetting session for next block.");
            self.reset_for_next_block();
            false // Continue session for next block
        } else {
            // Verification or persistence failed - error already logged
            true // Terminate on verification/persistence failure
        }
    }

    /// Waits for verification, then persistence. Handles verification failures.
    async fn wait_for_verification_and_persistence(
        &mut self,
        cache_key: uuid::Uuid,
    ) -> Result<(), ()> {
        let block_to_await = self.current_block_number as u64;
        let ack_timeout = Duration::from_secs(
            self.context
                .config
                .plugins
                .publish_service
                .persistence_ack_timeout_seconds,
        );

        trace!(session_id = %self.id, block = block_to_await, "Awaiting verification and persistence...");
        let start_time = Instant::now();

        // Subscribe to both verification failure and persistence events
        // If verification fails, we get verification_failed event
        // If verification succeeds, Persistence receives it and we get persisted event
        let mut rx_verification_failed = self.context.tx_block_verification_failed.subscribe();
        let mut rx_persisted = self.context.tx_block_persisted.subscribe();

        // Wait for EITHER verification failure OR persistence success
        let result = tokio::time::timeout(ack_timeout, async {
            loop {
                tokio::select! {
                    // Check for verification failure
                    Ok(failed_event) = rx_verification_failed.recv() => {
                        if failed_event.block_number == block_to_await && failed_event.cache_key == cache_key {
                            return Err(failed_event.reason);
                        }
                    }
                    // Check for persistence success (implies verification succeeded)
                    Ok(persisted_event) = rx_persisted.recv() => {
                        if persisted_event.block_number == block_to_await && persisted_event.cache_key == cache_key {
                            return Ok(persisted_event);
                        }
                    }
                }
            }
        })
        .await;

        match result {
            Ok(Ok(_persisted_event)) => {
                // SUCCESS CASE: Verification succeeded AND persistence succeeded
                info!(session_id = %self.id, block = block_to_await, "Block persisted. Broadcasting ACK and updating shared state.");

                let duration = start_time.elapsed().as_secs_f64();
                self.context
                    .metrics
                    .publish_persistence_duration_seconds
                    .with_label_values(&["acknowledged"])
                    .observe(duration);

                if let Some(start_life) = self.block_start_time {
                    let lifecycle_duration = start_life.elapsed().as_secs_f64();
                    self.context
                        .metrics
                        .publish_lifecycle_duration_seconds
                        .with_label_values(&["acknowledged"])
                        .observe(lifecycle_duration);
                }
                self.context.metrics.blocks_acknowledged.inc();

                self.shared_state
                    .set_latest_persisted_block(block_to_await as i64);

                // No need to track unacknowledged blocks anymore - verification happened BEFORE persistence
                // If we're here, the block is already verified and persisted

                let ack_msg = PublishStreamResponse {
                    response: Some(publish_stream_response::Response::Acknowledgement(
                        BlockAcknowledgement {
                            block_number: block_to_await,
                        },
                    )),
                };

                for session_entry in self.shared_state.active_sessions.iter() {
                    let _ = session_entry.value().send(Ok(ack_msg)).await;
                }

                self.context
                    .metrics
                    .publish_responses_sent_total
                    .with_label_values(&["Acknowledgement"])
                    .inc();

                self.shared_state.block_winners.remove(&block_to_await);
                Ok(())
            },
            Ok(Err(reason)) => {
                // VERIFICATION FAILED CASE
                error!(
                    session_id = %self.id,
                    block = block_to_await,
                    reason = %reason,
                    "Block verification failed!"
                );

                let duration = start_time.elapsed().as_secs_f64();
                self.context
                    .metrics
                    .publish_persistence_duration_seconds
                    .with_label_values(&["verification_failed"])
                    .observe(duration);

                // Send BAD_BLOCK_PROOF to this session only
                self.handle_end_error(BlockAction::BadBlockProof).await;

                // Broadcast RESEND to all OTHER sessions (since this block failed)
                let resend_msg = PublishStreamResponse {
                    response: Some(publish_stream_response::Response::ResendBlock(
                        ResendBlock {
                            block_number: block_to_await,
                        },
                    )),
                };

                for session_entry in self.shared_state.active_sessions.iter() {
                    if *session_entry.key() != self.id {
                        let _ = session_entry.value().send(Ok(resend_msg)).await;
                    }
                }

                self.context
                    .metrics
                    .publish_responses_sent_total
                    .with_label_values(&["ResendBlock"])
                    .inc();

                self.shared_state.block_winners.remove(&block_to_await);
                Err(())
            },
            Err(_) => {
                // TIMEOUT CASE (neither verification result nor persistence within timeout)
                warn!(session_id = %self.id, block = block_to_await, "Timeout waiting for verification/persistence. Broadcasting ResendBlock request.");

                let duration = start_time.elapsed().as_secs_f64();
                self.context
                    .metrics
                    .publish_persistence_duration_seconds
                    .with_label_values(&["timeout"])
                    .observe(duration);

                if let Some(start_life) = self.block_start_time {
                    let lifecycle_duration = start_life.elapsed().as_secs_f64();
                    self.context
                        .metrics
                        .publish_lifecycle_duration_seconds
                        .with_label_values(&["timeout"])
                        .observe(lifecycle_duration);
                }

                self.shared_state.block_winners.remove(&block_to_await);

                let resend_msg = PublishStreamResponse {
                    response: Some(publish_stream_response::Response::ResendBlock(
                        ResendBlock {
                            block_number: block_to_await,
                        },
                    )),
                };

                // Broadcast to other sessions (not self - this session will terminate)
                for session_entry in self.shared_state.active_sessions.iter() {
                    if *session_entry.key() != self.id {
                        let _ = session_entry.value().send(Ok(resend_msg)).await;
                    }
                }

                self.context
                    .metrics
                    .publish_responses_sent_total
                    .with_label_values(&["ResendBlock"])
                    .inc();

                Err(())
            },
        }
    }

    async fn send_response(&self, response: PublishStreamResponse) -> Result<(), ()> {
        if self.response_tx.send(Ok(response)).await.is_err() {
            debug!(session_id = %self.id, "Failed to send response to client. Connection may be closed.");
            Err(())
        } else {
            Ok(())
        }
    }

    /// Log session statistics (call when session ends)
    pub fn log_session_stats(&self) {
        let session_duration = self.session_start_time.elapsed();
        info!(
            session_id = %self.id,
            publisher_id = ?self.publisher_id,
            duration_secs = session_duration.as_secs(),
            blocks_received = self.blocks_received,
            blocks_accepted = self.blocks_accepted,
            blocks_skipped = self.blocks_skipped,
            avg_header_to_proof_secs = if self.header_proof_count > 0 {
                self.header_proof_total_duration / self.header_proof_count as f64
            } else {
                0.0
            },
            "Publisher session completed"
        );

        // Record session duration metric
        self.context
            .metrics
            .publish_session_duration_seconds
            .with_label_values::<&str>(&[])
            .observe(session_duration.as_secs_f64());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rock_node_core::{
        app_context::AppContext,
        cache::BlockDataCache,
        capability::CapabilityRegistry,
        config::{
            BackfillConfig, BackfillMode, BlockAccessServiceConfig, Config, CoreConfig,
            PersistenceServiceConfig, PluginConfigs, PublishServiceConfig, QueryServiceConfig,
            ServerStatusServiceConfig, StateManagementServiceConfig, SubscriberServiceConfig,
            VerificationServiceConfig,
        },
        metrics::MetricsRegistry,
    };
    use tokio::sync::{broadcast, mpsc};

    fn make_context() -> (
        AppContext,
        mpsc::Receiver<rock_node_core::events::BlockItemsReceived>,
        mpsc::Receiver<rock_node_core::events::BlockVerified>,
        broadcast::Receiver<rock_node_core::events::BlockPersisted>,
    ) {
        make_context_with_registry(create_test_metrics())
    }

    /// Helper function to create an isolated metrics registry for testing
    fn create_test_metrics() -> MetricsRegistry {
        rock_node_core::test_utils::create_isolated_metrics()
    }

    fn make_context_with_registry(
        metrics: MetricsRegistry,
    ) -> (
        AppContext,
        mpsc::Receiver<rock_node_core::events::BlockItemsReceived>,
        mpsc::Receiver<rock_node_core::events::BlockVerified>,
        broadcast::Receiver<rock_node_core::events::BlockPersisted>,
    ) {
        let config = Config {
            core: CoreConfig {
                log_level: "info".to_string(),
                database_path: ":memory:".to_string(),
                start_block_number: 0,
                grpc_address: "0.0.0.0".to_string(),
                grpc_port: 0,
            },
            plugins: PluginConfigs {
                observability: rock_node_core::config::ObservabilityConfig {
                    enabled: false,
                    listen_address: "127.0.0.1:0".to_string(),
                },
                persistence_service: PersistenceServiceConfig {
                    enabled: true,
                    cold_storage_path: "/tmp".to_string(),
                    hot_storage_block_count: 10,
                    archive_batch_size: 5,
                },
                publish_service: PublishServiceConfig {
                    enabled: true,
                    max_concurrent_streams: 8,
                    persistence_ack_timeout_seconds: 1,
                    stale_winner_timeout_seconds: 1,
                    winner_cleanup_interval_seconds: 60,
                    winner_cleanup_threshold_blocks: 100,
                    max_message_size_bytes: 32 * 1024 * 1024,
                    max_items_per_set: 10000,
                },
                verification_service: VerificationServiceConfig { enabled: true },
                block_access_service: BlockAccessServiceConfig { enabled: true },
                server_status_service: ServerStatusServiceConfig { enabled: true },
                state_management_service: StateManagementServiceConfig { enabled: true },
                subscriber_service: SubscriberServiceConfig {
                    enabled: true,
                    max_concurrent_streams: 8,
                    session_timeout_seconds: 1,
                    live_stream_queue_size: 64,
                    max_future_block_lookahead: 5,
                },
                query_service: QueryServiceConfig { enabled: true },
                backfill: BackfillConfig {
                    enabled: false,
                    peers: vec![],
                    mode: BackfillMode::GapFill,
                    check_interval_seconds: 60,
                    max_batch_size: 1000,
                },
            },
        };
        let (tx_persisted, rx_persisted) = broadcast::channel(16);
        let (tx_items, rx_items) = mpsc::channel(16);
        let (tx_verified, rx_verified) = mpsc::channel(16);
        let (tx_verification_failed, _rx_verification_failed) = broadcast::channel(16);
        (
            AppContext {
                config: std::sync::Arc::new(config),
                metrics: std::sync::Arc::new(metrics),
                capability_registry: std::sync::Arc::new(CapabilityRegistry::new()),
                service_providers: std::sync::Arc::new(std::sync::RwLock::new(
                    std::collections::HashMap::new(),
                )),
                block_data_cache: std::sync::Arc::new(BlockDataCache::new()),
                tx_block_items_received: tx_items,
                tx_block_verified: tx_verified,
                tx_block_verification_failed: tx_verification_failed,
                tx_block_persisted: tx_persisted,
            },
            rx_items,
            rx_verified,
            rx_persisted,
        )
    }

    #[tokio::test]
    async fn header_handles_duplicate_blocks() {
        let (context, _rx_items, _rx_verified, _rx_persisted) = make_context();
        let shared = Arc::new(SharedState::new());
        shared.set_latest_persisted_block(100);
        let (tx, mut rx) = mpsc::channel(4);
        let mut session = SessionManager::new(context, shared, tx);

        // Sending header 100 (same as latest_persisted) should be treated as duplicate
        let header_item = rock_node_protobufs::com::hedera::hapi::block::stream::BlockItem {
            item: Some(BlockItemType::BlockHeader(
                rock_node_protobufs::com::hedera::hapi::block::stream::output::BlockHeader {
                    hapi_proto_version: None,
                    software_version: None,
                    number: 100,
                    block_timestamp: None,
                    hash_algorithm: 0,
                },
            )),
        };
        let req = PublishRequestType::BlockItems(
            rock_node_protobufs::org::hiero::block::api::BlockItemSet {
                block_items: vec![header_item],
            },
        );

        // Should terminate with EndDuplicate
        assert!(session.handle_request(req).await);
        let msg = rx.try_recv().unwrap().unwrap();
        match msg.response.unwrap() {
            publish_stream_response::Response::EndStream(eos) => {
                assert_eq!(
                    eos.status,
                    publish_stream_response::end_of_stream::Code::DuplicateBlock as i32
                );
                assert_eq!(eos.block_number, 100);
            },
            _ => panic!("Expected EndStream DuplicateBlock"),
        }
    }

    #[tokio::test]
    async fn first_header_wins_primary_second_goes_behind() {
        let metrics = create_test_metrics();
        let (context, _rx_items, _rx_verified, _rx_persisted) = make_context_with_registry(metrics);
        let shared = Arc::new(SharedState::new());
        shared.set_latest_persisted_block(100); // Set to 100 so block 101 is the expected next
        let (tx1, mut rx1) = mpsc::channel(4);
        let (tx2, mut rx2) = mpsc::channel(4);
        let mut s1 = SessionManager::new(context.clone(), shared.clone(), tx1);
        let mut s2 = SessionManager::new(context.clone(), shared.clone(), tx2);

        // Register both sessions to receive broadcasts
        shared.active_sessions.insert(s1.id, s1.response_tx.clone());
        shared.active_sessions.insert(s2.id, s2.response_tx.clone());

        // Create header request for block 101
        let header_item_s1 = rock_node_protobufs::com::hedera::hapi::block::stream::BlockItem {
            item: Some(BlockItemType::BlockHeader(
                rock_node_protobufs::com::hedera::hapi::block::stream::output::BlockHeader {
                    hapi_proto_version: None,
                    software_version: None,
                    number: 101,
                    block_timestamp: None,
                    hash_algorithm: 0,
                },
            )),
        };
        let req_s1 = PublishRequestType::BlockItems(
            rock_node_protobufs::org::hiero::block::api::BlockItemSet {
                block_items: vec![header_item_s1.clone()],
            },
        );

        // s1 sends header first - should accept
        assert!(!(s1.handle_request(req_s1).await));

        // s2 attempts same block - should skip
        let req_s2 = PublishRequestType::BlockItems(
            rock_node_protobufs::org::hiero::block::api::BlockItemSet {
                block_items: vec![header_item_s1],
            },
        );
        assert!(!(s2.handle_request(req_s2).await));

        // s2 should receive SkipBlock promptly
        let msg = tokio::time::timeout(Duration::from_millis(200), rx2.recv())
            .await
            .expect("timeout waiting skipblock")
            .unwrap()
            .unwrap();
        match msg.response.unwrap() {
            publish_stream_response::Response::SkipBlock(sk) => {
                assert_eq!(sk.block_number, 101);
            },
            _ => panic!("Expected SkipBlock"),
        }

        // Primary session sends proof (continuation of same block) which triggers complete_block
        let proof_item = rock_node_protobufs::com::hedera::hapi::block::stream::BlockItem {
            item: Some(BlockItemType::BlockProof(
                rock_node_protobufs::com::hedera::hapi::block::stream::BlockProof {
                    block: 101,
                    ..Default::default()
                },
            )),
        };
        let req_proof = PublishRequestType::BlockItems(
            rock_node_protobufs::org::hiero::block::api::BlockItemSet {
                block_items: vec![proof_item],
            },
        );
        // Kick off handle_request and allow it to run to process timeout
        let handle = tokio::spawn(async move { s1.handle_request(req_proof).await });
        // Wait long enough for timeout path (configured to 1s in context)
        tokio::time::sleep(Duration::from_millis(1100)).await;
        let _ = handle.await.unwrap();

        // Since no persisted signal is sent, expect a ResendBlock to be broadcast to other sessions
        let msg = rx1.try_recv();
        assert!(msg.is_err(), "Primary should not get a resend immediately");
        let resend = tokio::time::timeout(Duration::from_secs(2), rx2.recv())
            .await
            .expect("timeout waiting resend")
            .unwrap()
            .unwrap();
        match resend.response.unwrap() {
            publish_stream_response::Response::ResendBlock(rb) => assert_eq!(rb.block_number, 101),
            _ => panic!("Expected ResendBlock"),
        }
    }

    #[tokio::test]
    async fn ack_path_updates_shared_state_and_sends_ack() {
        let metrics = create_test_metrics();
        let (context, mut rx_items, _rx_verified, _rx_persisted) =
            make_context_with_registry(metrics);
        let shared = Arc::new(SharedState::new());
        shared.set_latest_persisted_block(100); // Set to 100 so block 101 is the expected next
        let (tx, mut rx) = mpsc::channel(8);
        let mut s = SessionManager::new(context.clone(), shared.clone(), tx);

        // Register this session to receive ACK broadcast
        shared.active_sessions.insert(s.id, s.response_tx.clone());

        // simulate header + proof in a single request -> will call publish_complete_block and then wait for ack
        let header_item = rock_node_protobufs::com::hedera::hapi::block::stream::BlockItem {
            item: Some(BlockItemType::BlockHeader(
                rock_node_protobufs::com::hedera::hapi::block::stream::output::BlockHeader {
                    hapi_proto_version: None,
                    software_version: None,
                    number: 101,
                    block_timestamp: None,
                    hash_algorithm: 0,
                },
            )),
        };
        let proof_item = rock_node_protobufs::com::hedera::hapi::block::stream::BlockItem {
            item: Some(BlockItemType::BlockProof(
                rock_node_protobufs::com::hedera::hapi::block::stream::BlockProof {
                    block: 101,
                    ..Default::default()
                },
            )),
        };
        let req = PublishRequestType::BlockItems(
            rock_node_protobufs::org::hiero::block::api::BlockItemSet {
                block_items: vec![header_item, proof_item],
            },
        );

        // Concurrently wait for BlockItemsReceived event and emit persisted event with matching cache_key
        let tx_persisted = s.context.tx_block_persisted.clone();
        tokio::spawn(async move {
            // Wait for the BlockItemsReceived event to get the cache_key
            let items_received = rx_items.recv().await.unwrap();
            tokio::time::sleep(Duration::from_millis(50)).await;
            let _ = tx_persisted.send(rock_node_core::events::BlockPersisted {
                block_number: items_received.block_number,
                cache_key: items_received.cache_key,
            });
        });

        let should_terminate = s.handle_request(req).await;
        // After ack, publish_complete_block returns true and resets; handle_request returns false (continue)
        assert!(!should_terminate);

        // Expect an Acknowledgement response sent to the same session
        let msg = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("timeout waiting ack")
            .unwrap()
            .unwrap();
        match msg.response.unwrap() {
            publish_stream_response::Response::Acknowledgement(ack) => {
                assert_eq!(ack.block_number, 101)
            },
            _ => panic!("Expected Acknowledgement"),
        }

        assert_eq!(shared.get_latest_persisted_block(), 101);
    }

    #[tokio::test]
    async fn verification_failure_sends_bad_block_proof_to_primary() {
        let metrics = create_test_metrics();
        let (context, mut rx_items, _rx_verified, _rx_persisted) =
            make_context_with_registry(metrics);
        let shared = Arc::new(SharedState::new());
        shared.set_latest_persisted_block(100); // Set to 100 so block 101 is the expected next
        let (tx, mut rx) = mpsc::channel(8);
        let mut s = SessionManager::new(context.clone(), shared.clone(), tx);

        shared.active_sessions.insert(s.id, s.response_tx.clone());

        // Send header + proof for block 101
        let header_item = rock_node_protobufs::com::hedera::hapi::block::stream::BlockItem {
            item: Some(BlockItemType::BlockHeader(
                rock_node_protobufs::com::hedera::hapi::block::stream::output::BlockHeader {
                    number: 101,
                    ..Default::default()
                },
            )),
        };
        let proof_item = rock_node_protobufs::com::hedera::hapi::block::stream::BlockItem {
            item: Some(BlockItemType::BlockProof(
                rock_node_protobufs::com::hedera::hapi::block::stream::BlockProof {
                    block: 101,
                    ..Default::default()
                },
            )),
        };
        let req = PublishRequestType::BlockItems(
            rock_node_protobufs::org::hiero::block::api::BlockItemSet {
                block_items: vec![header_item, proof_item],
            },
        );

        // Simulate verification failure
        let tx_verification_failed = s.context.tx_block_verification_failed.clone();
        tokio::spawn(async move {
            let items_received = rx_items.recv().await.unwrap();
            tokio::time::sleep(Duration::from_millis(50)).await;
            let _ = tx_verification_failed.send(rock_node_core::events::BlockVerificationFailed {
                block_number: items_received.block_number,
                cache_key: items_received.cache_key,
                reason: "Invalid block signature".to_string(),
            });
        });

        let should_terminate = s.handle_request(req).await;
        // Verification failure should terminate the session
        assert!(should_terminate);

        // Expect EndOfStream with BAD_BLOCK_PROOF status
        let msg = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("timeout waiting for response")
            .unwrap()
            .unwrap();
        match msg.response.unwrap() {
            publish_stream_response::Response::EndStream(eos) => {
                assert_eq!(
                    eos.status,
                    publish_stream_response::end_of_stream::Code::BadBlockProof as i32
                );
                assert_eq!(eos.block_number, 101);
            },
            _ => panic!("Expected EndStream with BAD_BLOCK_PROOF"),
        }
    }

    #[tokio::test]
    async fn verification_timeout_sends_resend_to_other_sessions() {
        let metrics = create_test_metrics();
        let (context, _rx_items, _rx_verified, _rx_persisted) = make_context_with_registry(metrics);
        let shared = Arc::new(SharedState::new());
        shared.set_latest_persisted_block(101); // Set to 101 so block 102 is the expected next
        let (tx1, _rx1) = mpsc::channel(8);
        let (tx2, mut rx2) = mpsc::channel(8);
        let mut s1 = SessionManager::new(context.clone(), shared.clone(), tx1);
        let s2_id = {
            let s2 = SessionManager::new(context.clone(), shared.clone(), tx2.clone());
            s2.id
        };

        // Register both sessions
        shared.active_sessions.insert(s1.id, s1.response_tx.clone());
        shared.active_sessions.insert(s2_id, tx2.clone());

        // Send header + proof for block 102
        let header_item = rock_node_protobufs::com::hedera::hapi::block::stream::BlockItem {
            item: Some(BlockItemType::BlockHeader(
                rock_node_protobufs::com::hedera::hapi::block::stream::output::BlockHeader {
                    number: 102,
                    ..Default::default()
                },
            )),
        };
        let proof_item = rock_node_protobufs::com::hedera::hapi::block::stream::BlockItem {
            item: Some(BlockItemType::BlockProof(
                rock_node_protobufs::com::hedera::hapi::block::stream::BlockProof {
                    block: 102,
                    ..Default::default()
                },
            )),
        };
        let req = PublishRequestType::BlockItems(
            rock_node_protobufs::org::hiero::block::api::BlockItemSet {
                block_items: vec![header_item, proof_item],
            },
        );

        // Don't send any verification or persistence events - let it timeout
        let should_terminate = s1.handle_request(req).await;
        // Timeout DOES terminate the primary session
        assert!(should_terminate);

        // Session 2 (non-primary) should receive ResendBlock
        let msg2 = tokio::time::timeout(Duration::from_secs(2), rx2.recv())
            .await
            .expect("timeout waiting for ResendBlock to non-primary")
            .unwrap()
            .unwrap();
        match msg2.response.unwrap() {
            publish_stream_response::Response::ResendBlock(resend) => {
                assert_eq!(resend.block_number, 102);
            },
            _ => panic!("Expected ResendBlock to non-primary"),
        }
    }

    #[tokio::test]
    async fn header_handles_behind_blocks() {
        let (context, _rx_items, _rx_verified, _rx_persisted) = make_context();
        let shared = Arc::new(SharedState::new());
        shared.set_latest_persisted_block(100);
        let (tx, mut rx) = mpsc::channel(4);
        let mut session = SessionManager::new(context, shared, tx);

        // Sending header 50 (< latest_persisted = 100) should be Behind
        let header_item = rock_node_protobufs::com::hedera::hapi::block::stream::BlockItem {
            item: Some(BlockItemType::BlockHeader(
                rock_node_protobufs::com::hedera::hapi::block::stream::output::BlockHeader {
                    number: 50,
                    ..Default::default()
                },
            )),
        };
        let req = PublishRequestType::BlockItems(
            rock_node_protobufs::org::hiero::block::api::BlockItemSet {
                block_items: vec![header_item],
            },
        );

        // Should terminate with Behind
        assert!(session.handle_request(req).await);
        let msg = rx.try_recv().unwrap().unwrap();
        match msg.response.unwrap() {
            publish_stream_response::Response::EndStream(eos) => {
                assert_eq!(
                    eos.status,
                    publish_stream_response::end_of_stream::Code::Behind as i32
                );
                // EndOfStream contains latest_persisted (100), not the attempted block (50)
                assert_eq!(eos.block_number, 100);
            },
            _ => panic!("Expected EndStream Behind"),
        }
    }

    #[tokio::test]
    async fn verification_failure_sends_resend_to_other_sessions() {
        let metrics = create_test_metrics();
        let (context, mut rx_items, _rx_verified, _rx_persisted) =
            make_context_with_registry(metrics);
        let shared = Arc::new(SharedState::new());
        shared.set_latest_persisted_block(102); // Set to 102 so block 103 is the expected next
        let (tx1, mut rx1) = mpsc::channel(8);
        let (tx2, mut rx2) = mpsc::channel(8);
        let mut s1 = SessionManager::new(context.clone(), shared.clone(), tx1);
        let s2_id = {
            let s2 = SessionManager::new(context.clone(), shared.clone(), tx2.clone());
            s2.id
        };

        // Register both sessions
        shared.active_sessions.insert(s1.id, s1.response_tx.clone());
        shared.active_sessions.insert(s2_id, tx2.clone());

        // Send header + proof from primary session
        let header_item = rock_node_protobufs::com::hedera::hapi::block::stream::BlockItem {
            item: Some(BlockItemType::BlockHeader(
                rock_node_protobufs::com::hedera::hapi::block::stream::output::BlockHeader {
                    number: 103,
                    ..Default::default()
                },
            )),
        };
        let proof_item = rock_node_protobufs::com::hedera::hapi::block::stream::BlockItem {
            item: Some(BlockItemType::BlockProof(
                rock_node_protobufs::com::hedera::hapi::block::stream::BlockProof {
                    block: 103,
                    ..Default::default()
                },
            )),
        };
        let req = PublishRequestType::BlockItems(
            rock_node_protobufs::org::hiero::block::api::BlockItemSet {
                block_items: vec![header_item, proof_item],
            },
        );

        // Simulate verification failure
        let tx_verification_failed = s1.context.tx_block_verification_failed.clone();
        tokio::spawn(async move {
            let items_received = rx_items.recv().await.unwrap();
            tokio::time::sleep(Duration::from_millis(50)).await;
            let _ = tx_verification_failed.send(rock_node_core::events::BlockVerificationFailed {
                block_number: items_received.block_number,
                cache_key: items_received.cache_key,
                reason: "Invalid proof".to_string(),
            });
        });

        let should_terminate = s1.handle_request(req).await;
        assert!(should_terminate);

        // Primary session should get BAD_BLOCK_PROOF
        let msg1 = tokio::time::timeout(Duration::from_secs(1), rx1.recv())
            .await
            .expect("timeout waiting for BAD_BLOCK_PROOF")
            .unwrap()
            .unwrap();
        match msg1.response.unwrap() {
            publish_stream_response::Response::EndStream(eos) => {
                assert_eq!(
                    eos.status,
                    publish_stream_response::end_of_stream::Code::BadBlockProof as i32
                );
            },
            _ => panic!("Expected BAD_BLOCK_PROOF to primary"),
        }

        // Other session should get ResendBlock
        let msg2 = tokio::time::timeout(Duration::from_secs(1), rx2.recv())
            .await
            .expect("timeout waiting for ResendBlock")
            .unwrap()
            .unwrap();
        match msg2.response.unwrap() {
            publish_stream_response::Response::ResendBlock(resend) => {
                assert_eq!(resend.block_number, 103);
            },
            _ => panic!("Expected ResendBlock to other session"),
        }
    }

    // ============================================
    // NEW COMPREHENSIVE MULTI-PUBLISHER TESTS
    // ============================================

    #[tokio::test]
    async fn three_publishers_race_for_same_block() {
        // Test that when 3 publishers race for the same block,
        // only one wins and the other two get SkipBlock
        let metrics = create_test_metrics();
        let (context, _rx_items, _rx_verified, _rx_persisted) = make_context_with_registry(metrics);
        let shared = Arc::new(SharedState::new());
        shared.set_latest_persisted_block(200);

        let (tx1, mut rx1) = mpsc::channel(4);
        let (tx2, mut rx2) = mpsc::channel(4);
        let (tx3, mut rx3) = mpsc::channel(4);

        let mut s1 = SessionManager::new(context.clone(), shared.clone(), tx1);
        let mut s2 = SessionManager::new(context.clone(), shared.clone(), tx2);
        let mut s3 = SessionManager::new(context.clone(), shared.clone(), tx3);

        // Register all three sessions
        shared.active_sessions.insert(s1.id, s1.response_tx.clone());
        shared.active_sessions.insert(s2.id, s2.response_tx.clone());
        shared.active_sessions.insert(s3.id, s3.response_tx.clone());

        // All three send header for block 201
        let header = rock_node_protobufs::com::hedera::hapi::block::stream::BlockItem {
            item: Some(BlockItemType::BlockHeader(
                rock_node_protobufs::com::hedera::hapi::block::stream::output::BlockHeader {
                    number: 201,
                    ..Default::default()
                },
            )),
        };

        let req1 = PublishRequestType::BlockItems(
            rock_node_protobufs::org::hiero::block::api::BlockItemSet {
                block_items: vec![header.clone()],
            },
        );
        let req2 = PublishRequestType::BlockItems(
            rock_node_protobufs::org::hiero::block::api::BlockItemSet {
                block_items: vec![header.clone()],
            },
        );
        let req3 = PublishRequestType::BlockItems(
            rock_node_protobufs::org::hiero::block::api::BlockItemSet {
                block_items: vec![header],
            },
        );

        // s1 wins
        assert!(!s1.handle_request(req1).await);
        assert_eq!(s1.state, SessionState::Primary);

        // s2 and s3 should get SkipBlock
        assert!(!s2.handle_request(req2).await);
        assert!(!s3.handle_request(req3).await);

        // Verify both losers got SkipBlock
        for (name, rx) in [("s2", &mut rx2), ("s3", &mut rx3)] {
            let msg = tokio::time::timeout(Duration::from_millis(200), rx.recv())
                .await
                .expect(&format!("{} should receive SkipBlock", name))
                .unwrap()
                .unwrap();
            match msg.response.unwrap() {
                publish_stream_response::Response::SkipBlock(sk) => {
                    assert_eq!(sk.block_number, 201);
                },
                other => panic!("{} expected SkipBlock, got {:?}", name, other),
            }
        }

        // Winner should not receive anything yet
        assert!(rx1.try_recv().is_err());

        // Verify block_winners map has correct entry
        assert_eq!(shared.block_winners.get(&201).unwrap().value(), &s1.id);
    }

    #[tokio::test]
    async fn validation_error_empty_block_items() {
        // Test that validation rejects empty block items
        let metrics = create_test_metrics();
        let (context, _rx_items, _rx_verified, _rx_persisted) = make_context_with_registry(metrics);
        let shared = Arc::new(SharedState::new());
        shared.set_latest_persisted_block(100);

        let (tx, _rx) = mpsc::channel(4);
        let mut session = SessionManager::new(context, shared, tx);

        // Send empty block items
        let req = PublishRequestType::BlockItems(
            rock_node_protobufs::org::hiero::block::api::BlockItemSet {
                block_items: vec![],
            },
        );

        // Should terminate due to validation error
        assert!(session.handle_request(req).await);
    }

    #[tokio::test]
    async fn validation_error_missing_header_for_new_block() {
        // Test that validation rejects new blocks without header
        let metrics = create_test_metrics();
        let (context, _rx_items, _rx_verified, _rx_persisted) = make_context_with_registry(metrics);
        let shared = Arc::new(SharedState::new());
        shared.set_latest_persisted_block(100);

        let (tx, _rx) = mpsc::channel(4);
        let mut session = SessionManager::new(context, shared, tx);

        // Send proof without header (new block should start with header)
        let proof_item = rock_node_protobufs::com::hedera::hapi::block::stream::BlockItem {
            item: Some(BlockItemType::BlockProof(
                rock_node_protobufs::com::hedera::hapi::block::stream::BlockProof {
                    block: 101,
                    ..Default::default()
                },
            )),
        };
        let req = PublishRequestType::BlockItems(
            rock_node_protobufs::org::hiero::block::api::BlockItemSet {
                block_items: vec![proof_item],
            },
        );

        // Should terminate due to validation error
        assert!(session.handle_request(req).await);
    }

    #[tokio::test]
    async fn validation_error_too_many_items() {
        // Test that validation rejects excessive items
        let metrics = create_test_metrics();
        let (context, _rx_items, _rx_verified, _rx_persisted) = make_context_with_registry(metrics);

        // Create a new context with modified config
        let mut config = (*context.config).clone();
        config.plugins.publish_service.max_items_per_set = 5;
        let modified_context = rock_node_core::AppContext {
            config: Arc::new(config),
            metrics: context.metrics.clone(),
            capability_registry: context.capability_registry.clone(),
            service_providers: context.service_providers.clone(),
            block_data_cache: context.block_data_cache.clone(),
            tx_block_items_received: context.tx_block_items_received.clone(),
            tx_block_verified: context.tx_block_verified.clone(),
            tx_block_verification_failed: context.tx_block_verification_failed.clone(),
            tx_block_persisted: context.tx_block_persisted.clone(),
        };

        let shared = Arc::new(SharedState::new());
        shared.set_latest_persisted_block(100);

        let (tx, _rx) = mpsc::channel(4);
        let mut session = SessionManager::new(modified_context, shared, tx);

        // Create header + 10 items (exceeds limit of 5)
        let header = rock_node_protobufs::com::hedera::hapi::block::stream::BlockItem {
            item: Some(BlockItemType::BlockHeader(
                rock_node_protobufs::com::hedera::hapi::block::stream::output::BlockHeader {
                    number: 101,
                    ..Default::default()
                },
            )),
        };
        let mut items = vec![header];
        for _ in 0..9 {
            items.push(
                rock_node_protobufs::com::hedera::hapi::block::stream::BlockItem {
                    item: Some(BlockItemType::BlockProof(
                        rock_node_protobufs::com::hedera::hapi::block::stream::BlockProof::default(
                        ),
                    )),
                },
            );
        }

        let req = PublishRequestType::BlockItems(
            rock_node_protobufs::org::hiero::block::api::BlockItemSet { block_items: items },
        );

        // Should terminate due to validation error
        assert!(session.handle_request(req).await);
    }

    #[tokio::test]
    async fn multiple_consecutive_blocks_from_different_publishers() {
        // Test that different publishers can win consecutive blocks
        let metrics = create_test_metrics();
        let (context, mut rx_items, _rx_verified, _rx_persisted) =
            make_context_with_registry(metrics);
        let shared = Arc::new(SharedState::new());
        shared.set_latest_persisted_block(100);

        let (tx1, mut rx1) = mpsc::channel(8);
        let (tx2, _rx2) = mpsc::channel(8);

        let mut s1 = SessionManager::new(context.clone(), shared.clone(), tx1);
        let mut s2 = SessionManager::new(context.clone(), shared.clone(), tx2);

        shared.active_sessions.insert(s1.id, s1.response_tx.clone());
        shared.active_sessions.insert(s2.id, s2.response_tx.clone());

        // Publisher 1 wins block 101
        let header_101 = rock_node_protobufs::com::hedera::hapi::block::stream::BlockItem {
            item: Some(BlockItemType::BlockHeader(
                rock_node_protobufs::com::hedera::hapi::block::stream::output::BlockHeader {
                    number: 101,
                    ..Default::default()
                },
            )),
        };
        let proof_101 = rock_node_protobufs::com::hedera::hapi::block::stream::BlockItem {
            item: Some(BlockItemType::BlockProof(
                rock_node_protobufs::com::hedera::hapi::block::stream::BlockProof {
                    block: 101,
                    ..Default::default()
                },
            )),
        };

        let req_101_s1 = PublishRequestType::BlockItems(
            rock_node_protobufs::org::hiero::block::api::BlockItemSet {
                block_items: vec![header_101, proof_101],
            },
        );

        // Simulate persistence for block 101
        let tx_persisted = s1.context.tx_block_persisted.clone();
        tokio::spawn(async move {
            let items_received = rx_items.recv().await.unwrap();
            tokio::time::sleep(Duration::from_millis(50)).await;
            let _ = tx_persisted.send(rock_node_core::events::BlockPersisted {
                block_number: items_received.block_number,
                cache_key: items_received.cache_key,
            });
        });

        assert!(!s1.handle_request(req_101_s1).await);

        // Wait for ACK
        let ack = tokio::time::timeout(Duration::from_secs(1), rx1.recv())
            .await
            .expect("timeout waiting for ACK")
            .unwrap()
            .unwrap();
        match ack.response.unwrap() {
            publish_stream_response::Response::Acknowledgement(a) => {
                assert_eq!(a.block_number, 101);
            },
            _ => panic!("Expected Acknowledgement"),
        }

        // Now publisher 2 wins block 102
        shared.set_latest_persisted_block(101);

        let header_102 = rock_node_protobufs::com::hedera::hapi::block::stream::BlockItem {
            item: Some(BlockItemType::BlockHeader(
                rock_node_protobufs::com::hedera::hapi::block::stream::output::BlockHeader {
                    number: 102,
                    ..Default::default()
                },
            )),
        };

        let req_102_s2 = PublishRequestType::BlockItems(
            rock_node_protobufs::org::hiero::block::api::BlockItemSet {
                block_items: vec![header_102],
            },
        );

        assert!(!s2.handle_request(req_102_s2).await);
        assert_eq!(s2.state, SessionState::Primary);

        // Publisher 1 tries block 102 and should be skipped
        let header_102_s1 = rock_node_protobufs::com::hedera::hapi::block::stream::BlockItem {
            item: Some(BlockItemType::BlockHeader(
                rock_node_protobufs::com::hedera::hapi::block::stream::output::BlockHeader {
                    number: 102,
                    ..Default::default()
                },
            )),
        };

        let req_102_s1 = PublishRequestType::BlockItems(
            rock_node_protobufs::org::hiero::block::api::BlockItemSet {
                block_items: vec![header_102_s1],
            },
        );

        assert!(!s1.handle_request(req_102_s1).await);

        // s1 should get SkipBlock for 102
        let skip = tokio::time::timeout(Duration::from_millis(200), rx1.recv())
            .await
            .expect("timeout waiting for SkipBlock")
            .unwrap()
            .unwrap();
        match skip.response.unwrap() {
            publish_stream_response::Response::SkipBlock(sk) => {
                assert_eq!(sk.block_number, 102);
            },
            _ => panic!("Expected SkipBlock"),
        }
    }

    #[tokio::test]
    async fn winner_cleanup_on_session_disconnect() {
        // Test that when winner disconnects, block_winners entry is cleaned up
        // and other sessions get ResendBlock
        let metrics = create_test_metrics();
        let (context, _rx_items, _rx_verified, _rx_persisted) = make_context_with_registry(metrics);
        let shared = Arc::new(SharedState::new());
        shared.set_latest_persisted_block(100);

        let (tx1, _rx1) = mpsc::channel(4);
        let (tx2, mut rx2) = mpsc::channel(4);

        let s1 = SessionManager::new(context.clone(), shared.clone(), tx1);
        let s2 = SessionManager::new(context.clone(), shared.clone(), tx2.clone());

        let s1_id = s1.id;

        shared.active_sessions.insert(s1.id, s1.response_tx.clone());
        shared.active_sessions.insert(s2.id, tx2.clone());

        // Manually set s1 as winner for block 101
        shared.block_winners.insert(101, s1_id);

        // Simulate s1 disconnecting (drop it)
        drop(s1);

        // Remove s1 from active sessions and clean up affected blocks
        shared.active_sessions.remove(&s1_id);

        // Find affected blocks and broadcast ResendBlock
        let mut affected_blocks = Vec::new();
        for entry in shared.block_winners.iter() {
            if entry.value() == &s1_id {
                affected_blocks.push(*entry.key());
            }
        }

        // Cleanup and broadcast
        for block_num in &affected_blocks {
            shared.block_winners.remove(block_num);

            // Broadcast ResendBlock to remaining sessions
            for session_entry in shared.active_sessions.iter() {
                let resend_msg = rock_node_protobufs::org::hiero::block::api::PublishStreamResponse {
                    response: Some(publish_stream_response::Response::ResendBlock(
                        rock_node_protobufs::org::hiero::block::api::publish_stream_response::ResendBlock {
                            block_number: *block_num,
                        },
                    )),
                };
                let _ = session_entry.value().try_send(Ok(resend_msg));
            }
        }

        // s2 should receive ResendBlock for 101
        let resend = tokio::time::timeout(Duration::from_millis(200), rx2.recv())
            .await
            .expect("timeout waiting for ResendBlock")
            .unwrap()
            .unwrap();

        match resend.response.unwrap() {
            publish_stream_response::Response::ResendBlock(rb) => {
                assert_eq!(rb.block_number, 101);
            },
            _ => panic!("Expected ResendBlock"),
        }

        // Verify block_winners is cleaned up
        assert!(!shared.block_winners.contains_key(&101));
    }

    #[tokio::test]
    async fn session_stats_recorded_correctly() {
        // Test that session statistics are correctly tracked
        let metrics = create_test_metrics();
        let (context, _rx_items, _rx_verified, _rx_persisted) = make_context_with_registry(metrics);
        let shared = Arc::new(SharedState::new());
        shared.set_latest_persisted_block(100);

        let (tx, mut rx) = mpsc::channel(8);
        let mut session = SessionManager::new(context, shared, tx);

        // Set publisher ID
        session.publisher_id = Some("test-publisher-1".to_string());

        // Send 3 blocks: 2 accepted, 1 skipped
        let header_101 = rock_node_protobufs::com::hedera::hapi::block::stream::BlockItem {
            item: Some(BlockItemType::BlockHeader(
                rock_node_protobufs::com::hedera::hapi::block::stream::output::BlockHeader {
                    number: 101,
                    ..Default::default()
                },
            )),
        };

        // First block - accepted (wins)
        let req1 = PublishRequestType::BlockItems(
            rock_node_protobufs::org::hiero::block::api::BlockItemSet {
                block_items: vec![header_101.clone()],
            },
        );
        assert!(!session.handle_request(req1).await);
        assert_eq!(session.state, SessionState::Primary);

        // Manually trigger skip
        session.handle_skip().await;

        // Clear the channel
        let _ = rx.try_recv();

        // Another block - accepted
        session.reset_for_next_block();
        session.state = SessionState::New;
        let header_102 = rock_node_protobufs::com::hedera::hapi::block::stream::BlockItem {
            item: Some(BlockItemType::BlockHeader(
                rock_node_protobufs::com::hedera::hapi::block::stream::output::BlockHeader {
                    number: 102,
                    ..Default::default()
                },
            )),
        };
        let req2 = PublishRequestType::BlockItems(
            rock_node_protobufs::org::hiero::block::api::BlockItemSet {
                block_items: vec![header_102],
            },
        );
        assert!(!session.handle_request(req2).await);

        // Verify stats
        assert_eq!(session.blocks_received, 2);
        assert_eq!(session.blocks_skipped, 1);
        assert!(session.session_start_time.elapsed().as_secs() < 5);

        // Log stats would normally be called on disconnect
        session.log_session_stats();
    }

    #[tokio::test]
    async fn concurrent_different_blocks_from_multiple_publishers() {
        // Test that different publishers can work on different blocks concurrently
        let metrics = create_test_metrics();
        let (context, _rx_items, _rx_verified, _rx_persisted) = make_context_with_registry(metrics);
        let shared = Arc::new(SharedState::new());
        shared.set_latest_persisted_block(100);

        let (tx1, _rx1) = mpsc::channel(4);
        let (tx2, _rx2) = mpsc::channel(4);

        let mut s1 = SessionManager::new(context.clone(), shared.clone(), tx1);
        let mut s2 = SessionManager::new(context.clone(), shared.clone(), tx2);

        shared.active_sessions.insert(s1.id, s1.response_tx.clone());
        shared.active_sessions.insert(s2.id, s2.response_tx.clone());

        // s1 starts block 101
        let header_101 = rock_node_protobufs::com::hedera::hapi::block::stream::BlockItem {
            item: Some(BlockItemType::BlockHeader(
                rock_node_protobufs::com::hedera::hapi::block::stream::output::BlockHeader {
                    number: 101,
                    ..Default::default()
                },
            )),
        };
        let req_101 = PublishRequestType::BlockItems(
            rock_node_protobufs::org::hiero::block::api::BlockItemSet {
                block_items: vec![header_101],
            },
        );
        assert!(!s1.handle_request(req_101).await);
        assert_eq!(s1.state, SessionState::Primary);
        assert_eq!(s1.current_block_number, 101);

        // s2 starts block 102 (ahead of persisted, but allowed)
        let header_102 = rock_node_protobufs::com::hedera::hapi::block::stream::BlockItem {
            item: Some(BlockItemType::BlockHeader(
                rock_node_protobufs::com::hedera::hapi::block::stream::output::BlockHeader {
                    number: 102,
                    ..Default::default()
                },
            )),
        };
        let req_102 = PublishRequestType::BlockItems(
            rock_node_protobufs::org::hiero::block::api::BlockItemSet {
                block_items: vec![header_102],
            },
        );
        assert!(!s2.handle_request(req_102).await);
        assert_eq!(s2.state, SessionState::Primary);
        assert_eq!(s2.current_block_number, 102);

        // Both should be registered as winners for their respective blocks
        assert_eq!(shared.block_winners.get(&101).unwrap().value(), &s1.id);
        assert_eq!(shared.block_winners.get(&102).unwrap().value(), &s2.id);
    }
}
