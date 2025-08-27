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
    block_start_time: Option<Instant>,
    item_buffer: Vec<rock_node_protobufs::com::hedera::hapi::block::stream::BlockItem>,
    response_tx: mpsc::Sender<Result<PublishStreamResponse, Status>>,
    header_proof_total_duration: f64,
    header_proof_count: u64,
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
            block_start_time: None,
            item_buffer: Vec::new(),
            response_tx,
            header_proof_total_duration: 0.0,
            header_proof_count: 0,
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
                            // Measure header -> proof duration
                            if let Some(start) = self.block_start_time {
                                let duration = start.elapsed().as_secs_f64();
                                // Histogram (no labels)
                                self.context
                                    .metrics
                                    .publish_header_to_proof_duration_seconds
                                    .with_label_values::<&str>(&[])
                                    .observe(duration);
                                // Update running average for this session
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
            self.context
                .metrics
                .publish_blocks_received_total
                .with_label_values(&["duplicate"])
                .inc();
            let response = response_from_code(
                publish_stream_response::end_of_stream::Code::DuplicateBlock,
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
            info!(session_id = %self.id, block_number = self.current_block_number, "Block persisted. Resetting session for next block.");
            self.reset_for_next_block();
            true
        } else {
            false
        }
    }

    /// Waits for the persistence event and broadcasts the result to all active sessions.
    async fn wait_for_persistence_ack(&mut self) -> Result<(), ()> {
        let mut rx_persisted = self.context.tx_block_persisted.subscribe();
        let block_to_await = self.current_block_number;
        let ack_timeout = Duration::from_secs(
            self.context
                .config
                .plugins
                .publish_service
                .persistence_ack_timeout_seconds,
        );

        trace!(session_id = %self.id, block = block_to_await, "Awaiting persistence ACK...");
        let start_time = Instant::now();

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
                // SUCCESS CASE
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

                self.context
                    .metrics
                    .publish_responses_sent_total
                    .with_label_values(&["Acknowledgement"])
                    .inc();

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

                for session_entry in self.shared_state.active_sessions.iter() {
                    if *session_entry.key() != self.id {
                        let _ = session_entry.value().send(Ok(resend_msg.clone())).await;
                    }
                }

                self.context
                    .metrics
                    .publish_responses_sent_total
                    .with_label_values(&["ResendBlock"])
                    .inc();

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
        make_context_with_registry(MetricsRegistry::new().unwrap())
    }

    /// Helper function to create an isolated metrics registry for testing
    fn create_test_metrics() -> MetricsRegistry {
        // Create a fresh registry to avoid cardinality conflicts
        let registry = prometheus::Registry::new();
        MetricsRegistry::with_registry(registry).unwrap()
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
                    grpc_address: "127.0.0.1".to_string(),
                    grpc_port: 0,
                    max_concurrent_streams: 8,
                    persistence_ack_timeout_seconds: 1,
                    stale_winner_timeout_seconds: 1,
                    winner_cleanup_interval_seconds: 60,
                    winner_cleanup_threshold_blocks: 100,
                },
                verification_service: VerificationServiceConfig { enabled: true },
                block_access_service: BlockAccessServiceConfig {
                    enabled: true,
                    grpc_address: "127.0.0.1".to_string(),
                    grpc_port: 0,
                },
                server_status_service: ServerStatusServiceConfig {
                    enabled: true,
                    grpc_address: "127.0.0.1".to_string(),
                    grpc_port: 0,
                },
                state_management_service: StateManagementServiceConfig { enabled: true },
                subscriber_service: SubscriberServiceConfig {
                    enabled: true,
                    grpc_address: "127.0.0.1".to_string(),
                    grpc_port: 0,
                    max_concurrent_streams: 8,
                    session_timeout_seconds: 1,
                    live_stream_queue_size: 64,
                    max_future_block_lookahead: 5,
                },
                query_service: QueryServiceConfig {
                    enabled: true,
                    grpc_address: "127.0.0.1".to_string(),
                    grpc_port: 0,
                },
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

        // Sending header 100 should be treated as duplicate and emit EndStream
        assert_eq!(session.handle_block_header(100).await, false);
        let msg = rx.try_recv().unwrap().unwrap();
        match msg.response.unwrap() {
            publish_stream_response::Response::EndStream(eos) => {
                assert_eq!(
                    eos.status,
                    publish_stream_response::end_of_stream::Code::DuplicateBlock as i32
                );
                assert_eq!(eos.block_number, 100);
            }
            _ => panic!("Expected EndStream DuplicateBlock"),
        }
    }

    #[tokio::test]
    async fn first_header_wins_primary_second_goes_behind() {
        let metrics = create_test_metrics();
        let (context, _rx_items, _rx_verified, _rx_persisted) = make_context_with_registry(metrics);
        let shared = Arc::new(SharedState::new());
        let (tx1, mut rx1) = mpsc::channel(4);
        let (tx2, mut rx2) = mpsc::channel(4);
        let mut s1 = SessionManager::new(context.clone(), shared.clone(), tx1);
        let mut s2 = SessionManager::new(context.clone(), shared.clone(), tx2);

        // Register both sessions to receive broadcasts
        shared.active_sessions.insert(s1.id, s1.response_tx.clone());
        shared.active_sessions.insert(s2.id, s2.response_tx.clone());

        assert_eq!(s1.handle_block_header(101).await, true);
        // s2 attempts same block
        assert_eq!(s2.handle_block_header(101).await, true);

        // s2 should receive SkipBlock promptly
        let msg = tokio::time::timeout(Duration::from_millis(200), rx2.recv())
            .await
            .expect("timeout waiting skipblock")
            .unwrap()
            .unwrap();
        match msg.response.unwrap() {
            publish_stream_response::Response::SkipBlock(sk) => {
                assert_eq!(sk.block_number, 101);
            }
            _ => panic!("Expected SkipBlock"),
        }

        // Primary session accumulates items and then proof leads to publish
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
        // Kick off handle_request and allow it to run to process timeout
        let handle = tokio::spawn(async move { s1.handle_request(req).await });
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
        let (context, _rx_items, _rx_verified, _rx_persisted) = make_context_with_registry(metrics);
        let shared = Arc::new(SharedState::new());
        let (tx, mut rx) = mpsc::channel(8);
        let mut s = SessionManager::new(context.clone(), shared.clone(), tx);

        // Register this session to receive ACK broadcast
        shared.active_sessions.insert(s.id, s.response_tx.clone());

        // start header
        assert_eq!(s.handle_block_header(200).await, true);
        // simulate proof -> will call publish_complete_block and then wait for ack
        let header_item = rock_node_protobufs::com::hedera::hapi::block::stream::BlockItem {
            item: Some(BlockItemType::BlockHeader(
                rock_node_protobufs::com::hedera::hapi::block::stream::output::BlockHeader {
                    hapi_proto_version: None,
                    software_version: None,
                    number: 200,
                    block_timestamp: None,
                    hash_algorithm: 0,
                },
            )),
        };
        let proof_item = rock_node_protobufs::com::hedera::hapi::block::stream::BlockItem {
            item: Some(BlockItemType::BlockProof(
                rock_node_protobufs::com::hedera::hapi::block::stream::BlockProof {
                    block: 200,
                    ..Default::default()
                },
            )),
        };
        let req = PublishRequestType::BlockItems(
            rock_node_protobufs::org::hiero::block::api::BlockItemSet {
                block_items: vec![header_item, proof_item],
            },
        );

        // Concurrently emit persisted event after a short delay
        let tx_persisted = s.context.tx_block_persisted.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let _ = tx_persisted.send(rock_node_core::events::BlockPersisted {
                block_number: 200,
                cache_key: uuid::Uuid::new_v4(),
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
                assert_eq!(ack.block_number, 200)
            }
            _ => panic!("Expected Acknowledgement"),
        }

        assert_eq!(shared.get_latest_persisted_block(), 200);
    }
}
