use crate::error::SubscriberError;
use futures_util::FutureExt;
use prost::Message;
use rock_node_core::{app_context::AppContext, block_reader::BlockReader, service_provider::BlockReaderProvider};
use rock_node_protobufs::com::hedera::hapi::block::stream::Block;
use rock_node_protobufs::org::hiero::block::api::{
    subscribe_stream_response::{Code, Response as ResponseType},
    BlockItemSet, SubscribeStreamRequest, SubscribeStreamResponse,
};
use std::{any::TypeId, sync::Arc, time::Duration};
use tokio::sync::mpsc;
use tonic::Status;
use tracing::{debug, info, warn};
use uuid::Uuid;

pub struct SubscriberSession {
    pub id: Uuid,
    context: Arc<AppContext>,
    request: SubscribeStreamRequest,
    response_tx: mpsc::Sender<Result<SubscribeStreamResponse, Status>>,
    block_reader: Arc<dyn BlockReader>,
}

impl SubscriberSession {
    pub fn new(
        context: Arc<AppContext>,
        request: SubscribeStreamRequest,
        response_tx: mpsc::Sender<Result<SubscribeStreamResponse, Status>>,
    ) -> Self {
        let block_reader = context
            .service_providers
            .read()
            .unwrap()
            .get(&TypeId::of::<BlockReaderProvider>())
            .and_then(|p| p.downcast_ref::<BlockReaderProvider>())
            .map(|p| p.get_service())
            .expect("BlockReaderProvider not found in service providers!");

        Self {
            id: Uuid::new_v4(),
            context,
            request,
            response_tx,
            block_reader,
        }
    }

    /// Redesigned main entry point with a single, robust loop.
    pub async fn run(&mut self) {
        self.context.metrics.subscriber_active_sessions.inc();
        let _drop_guard = DropGuard::new(self.context.metrics.subscriber_active_sessions.clone());
        let session_counter = self.context.metrics.subscriber_sessions_total.clone();

        let mut final_code = Code::ReadStreamSuccess;
        let mut outcome_label = "completed";

        if let Err(e) = self.execute_stream().await {
            final_code = e.to_status_code();
            outcome_label = e.to_metric_label();
            warn!(session_id = %self.id, error = %e, "Stream execution ended with an error.");
        }
        
        session_counter.with_label_values(&[outcome_label]).inc();
        self.send_final_status(final_code).await;
    }

    /// The core state machine, redesigned into a single robust loop.
    async fn execute_stream(&mut self) -> Result<(), SubscriberError> {
        let mut next_block_to_send = self.validate_request().await?;
        let is_finite_stream = self.request.end_block_number != u64::MAX;

        // A single master loop that drives the entire session.
        loop {
            // Check for successful completion of a finite stream.
            if is_finite_stream && next_block_to_send > self.request.end_block_number {
                info!(session_id = %self.id, "Finite stream completed successfully.");
                return Ok(());
            }

            // --- Phase 1: Attempt to get the block from historical storage. ---
            match self.block_reader.read_block(next_block_to_send) {
                Ok(Some(block_bytes)) => {
                    // Success: The block was on disk. Send it and continue the loop.
                    self.send_block(&block_bytes).await?;
                    self.context.metrics.subscriber_blocks_sent_total.with_label_values(&["historical"]).inc();
                    next_block_to_send += 1;
                    continue; // Immediately try to get the next block.
                }
                Ok(None) => {
                    // The block is not on disk. We must wait for it to be persisted.
                    // Fall through to Phase 2.
                }
                Err(e) => return Err(SubscriberError::Persistence(e)),
            }

            // --- Phase 2: Wait for the needed block to be announced on the live event bus. ---
            debug!(session_id = %self.id, "Block #{} not in history, waiting for live event.", next_block_to_send);
            self.wait_for_block_persisted(next_block_to_send).await?;
            debug!(session_id = %self.id, "Event for block #{} received. Looping back to read from disk.", next_block_to_send);

            // After waiting, we loop back to the top to re-attempt the read from the BlockReader.
            // This ensures we always serve from the canonical, persisted source.
        }
    }

    /// Waits for a specific block number to be announced on the `BlockPersisted` event bus.
    async fn wait_for_block_persisted(&self, block_number_needed: u64) -> Result<(), SubscriberError> {
        let mut broadcast_rx = self.context.tx_block_persisted.subscribe();
        let timeout_duration = Duration::from_secs(30); // Timeout to prevent waiting forever.

        loop {
            match tokio::time::timeout(timeout_duration, broadcast_rx.recv().fuse()).await {
                Ok(Ok(event)) => {
                    if event.block_number == block_number_needed {
                        return Ok(()); // Found it!
                    }
                    if event.block_number > block_number_needed {
                        // The bus is already ahead of us. This means we lagged and missed the event.
                        return Err(SubscriberError::StreamLagged);
                    }
                    // It's an older block, ignore it and keep waiting.
                }
                Ok(Err(_)) => {
                    // The broadcast channel has lagged and closed the receiver.
                    return Err(SubscriberError::StreamLagged);
                }
                Err(_) => {
                    // The timeout elapsed.
                    return Err(SubscriberError::TimeoutWaitingForBlock(block_number_needed));
                }
            }
        }
    }

    async fn validate_request(&self) -> Result<u64, SubscriberError> {
        // Validation logic remains the same...
        let req_start = self.request.start_block_number;
        let req_end = self.request.end_block_number;
        let earliest = self.block_reader.get_earliest_persisted_block_number()?;
        let start_block = match req_start {
            u64::MAX => earliest.unwrap_or(0),
            literal_start => literal_start,
        };
        if req_end != u64::MAX && start_block > req_end {
            return Err(SubscriberError::Validation(format!("..."), Code::ReadStreamInvalidEndBlockNumber));
        }
        if let Some(earliest_num) = earliest {
            if start_block < earliest_num {
                return Err(SubscriberError::Validation(format!("..."), Code::ReadStreamInvalidStartBlockNumber));
            }
        }
        Ok(start_block)
    }

    async fn send_block(&self, block_bytes: &[u8]) -> Result<(), SubscriberError> {
        let item_set = BlockItemSet {
            block_items: Block::decode(block_bytes).map_err(|e| SubscriberError::Persistence(e.into()))?.items,
        };
        let response = SubscribeStreamResponse { response: Some(ResponseType::BlockItems(item_set)) };
        if self.response_tx.send(Ok(response)).await.is_err() {
            Err(SubscriberError::ClientDisconnected)
        } else {
            Ok(())
        }
    }
    
    async fn send_final_status(&self, code: Code) {
        if let Code::ReadStreamUnknown = code { return; }
        info!(session_id = %self.id, "SENDING FINAL STATUS: {:?}", code);
        let response = SubscribeStreamResponse { response: Some(ResponseType::Status(code.into())) };
        if self.response_tx.send(Ok(response)).await.is_err() {
            debug!(session_id = %self.id, "Client disconnected before final status could be sent.");
        }
    }
}

struct DropGuard { gauge: prometheus::IntGauge }
impl DropGuard { fn new(gauge: prometheus::IntGauge) -> Self { Self { gauge } } }
impl Drop for DropGuard { fn drop(&mut self) { self.gauge.dec(); } }
