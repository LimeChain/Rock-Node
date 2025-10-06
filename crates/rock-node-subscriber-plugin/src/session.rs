use crate::error::SubscriberError;
use prost::Message;
use rock_node_core::BlockReaderProvider;
use rock_node_core::{app_context::AppContext, block_reader::BlockReader};
use rock_node_protobufs::com::hedera::hapi::block::stream::Block;
use rock_node_protobufs::org::hiero::block::api::{
    subscribe_stream_response::{Code, Response as ResponseType},
    BlockItemSet, SubscribeStreamRequest, SubscribeStreamResponse,
};
use std::{
    any::TypeId,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, Notify};
use tonic::Status;
use tracing::{debug, info, warn};
use uuid::Uuid;

pub struct SubscriberSession {
    pub id: Uuid,
    context: Arc<AppContext>,
    request: SubscribeStreamRequest,
    response_tx: mpsc::Sender<Result<SubscribeStreamResponse, Status>>,
    block_reader: Arc<dyn BlockReader>,
    server_shutdown_notify: Arc<Notify>,
    session_shutdown_notify: Arc<Notify>,
    inter_block_metrics: Mutex<InterBlockMetrics>,
}

impl SubscriberSession {
    pub fn new(
        context: Arc<AppContext>,
        request: SubscribeStreamRequest,
        response_tx: mpsc::Sender<Result<SubscribeStreamResponse, Status>>,
        server_shutdown_notify: Arc<Notify>,
        session_shutdown_notify: Arc<Notify>,
    ) -> Result<Self, Status> {
        let block_reader = {
            let providers = context.service_providers.read().map_err(|_| {
                Status::internal("Failed to acquire read lock on service providers")
            })?;

            providers
                .get(&TypeId::of::<BlockReaderProvider>())
                .and_then(|p| p.downcast_ref::<BlockReaderProvider>())
                .map(|p_concrete| p_concrete.get_reader())
                .ok_or_else(|| Status::internal("BlockReaderProvider not found"))?
        };

        Ok(Self {
            id: Uuid::new_v4(),
            context,
            request,
            response_tx,
            block_reader,
            server_shutdown_notify,
            session_shutdown_notify,
            inter_block_metrics: Mutex::new(Default::default()),
        })
    }

    pub async fn run(&mut self) {
        self.context.metrics.subscriber_active_sessions.inc();
        let _drop_guard = DropGuard::new(self.context.metrics.subscriber_active_sessions.clone());

        let result = self.execute_stream().await;
        self.finalize_stream(result).await;
    }

    async fn execute_stream(&mut self) -> Result<(), SubscriberError> {
        let mut next_block_to_send = self.validate_request().await?;
        let is_finite_stream = self.request.end_block_number != u64::MAX;

        loop {
            tokio::select! {
                _ = self.server_shutdown_notify.notified() => {
                    return Err(SubscriberError::ServerShutdown);
                }
                _ = self.session_shutdown_notify.notified() => {
                    return Err(SubscriberError::ServerShutdown);
                }
                res = self.process_next_block(next_block_to_send, is_finite_stream) => {
                    match res {
                        Ok(Some(next_block)) => {
                            next_block_to_send = next_block;
                        }
                        Ok(None) => {
                            return Ok(());
                        }
                        Err(e) => {
                            return Err(e);
                        }
                    }
                }
            }
        }
    }

    async fn process_next_block(
        &self,
        block_to_send: u64,
        is_finite: bool,
    ) -> Result<Option<u64>, SubscriberError> {
        if is_finite && block_to_send > self.request.end_block_number {
            return Ok(None);
        }

        match self.block_reader.read_block(block_to_send) {
            Ok(Some(block_bytes)) => {
                self.send_block(&block_bytes).await?;
                self.context
                    .metrics
                    .subscriber_blocks_sent_total
                    .with_label_values(&["historical"])
                    .inc();
                return Ok(Some(block_to_send + 1));
            },
            Ok(None) => {},
            Err(e) => return Err(SubscriberError::Persistence(e)),
        }

        self.wait_for_new_block(block_to_send).await?;
        Ok(Some(block_to_send))
    }

    async fn wait_for_new_block(&self, block_number_needed: u64) -> Result<(), SubscriberError> {
        let mut broadcast_rx = self.context.tx_block_persisted.subscribe();
        let timeout_duration = Duration::from_secs(
            self.context
                .config
                .plugins
                .subscriber_service
                .session_timeout_seconds,
        );

        debug!(session_id = %self.id, "Waiting for block #{} or newer...", block_number_needed);
        loop {
            match tokio::time::timeout(timeout_duration, broadcast_rx.recv()).await {
                Ok(Ok(event)) => {
                    if event.block_number >= block_number_needed {
                        self.context
                            .metrics
                            .subscriber_blocks_sent_total
                            .with_label_values(&["live"])
                            .inc();
                        return Ok(());
                    }
                },
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {
                    warn!(session_id = %self.id, "Subscriber lagged behind event bus. Forcing persistence check.");
                    return Ok(());
                },
                Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => {
                    return Err(SubscriberError::Internal(
                        "Broadcast channel closed".to_string(),
                    ));
                },
                Err(_) => {
                    return Err(SubscriberError::TimeoutWaitingForBlock(block_number_needed));
                },
            }
        }
    }

    async fn validate_request(&self) -> Result<u64, SubscriberError> {
        let req_start = self.request.start_block_number;
        let req_end = self.request.end_block_number;
        let earliest = self.block_reader.get_earliest_persisted_block_number()?;
        let start_block = match req_start {
            u64::MAX => earliest.unwrap_or(0),
            literal_start => literal_start,
        };
        if req_end != u64::MAX && start_block > req_end {
            return Err(SubscriberError::Validation(
                format!(
                    "Start block {} cannot be after end block {}",
                    start_block, req_end
                ),
                Code::InvalidEndBlockNumber,
            ));
        }
        if let Some(earliest_num) = earliest {
            if start_block < earliest_num {
                return Err(SubscriberError::Validation(
                    format!(
                        "Requested start block {} is earlier than the first available block {}",
                        start_block, earliest_num
                    ),
                    Code::InvalidStartBlockNumber,
                ));
            }
        }
        Ok(start_block)
    }

    async fn send_block(&self, block_bytes: &[u8]) -> Result<(), SubscriberError> {
        let item_set = BlockItemSet {
            block_items: Block::decode(block_bytes)
                .map_err(|e| SubscriberError::Persistence(e.into()))?
                .items,
        };
        {
            // Update inter-block timing statistics
            let now = Instant::now();
            let mut metrics_lock = self.inter_block_metrics.lock().unwrap();
            if let Some(last) = metrics_lock.last_time {
                let delta = now.duration_since(last).as_secs_f64();
                metrics_lock.total_interval += delta;
                metrics_lock.interval_count += 1;
            }
            metrics_lock.last_time = Some(now);

            if metrics_lock.interval_count > 0 {
                let avg = metrics_lock.total_interval / metrics_lock.interval_count as f64;
                self.context
                    .metrics
                    .subscriber_average_inter_block_time_seconds
                    .with_label_values(&[&self.id.to_string()])
                    .set(avg);
            }
        }

        info!(session_id = %self.id, "Sending block #{:?} to client", item_set.block_items[0]);
        let response = SubscribeStreamResponse {
            response: Some(ResponseType::BlockItems(item_set)),
        };
        if self.response_tx.send(Ok(response)).await.is_err() {
            Err(SubscriberError::ClientDisconnected)
        } else {
            Ok(())
        }
    }

    async fn finalize_stream(&self, result: Result<(), SubscriberError>) {
        let (final_code, outcome_label) = match result {
            Ok(()) => (Code::Success, "completed"),
            Err(e) => (e.to_status_code(), e.to_metric_label()),
        };
        self.context
            .metrics
            .subscriber_sessions_total
            .with_label_values(&[outcome_label])
            .inc();

        // Record average inter-block time for this session
        {
            let metrics_lock = self.inter_block_metrics.lock().unwrap();
            if metrics_lock.interval_count > 0 {
                let avg = metrics_lock.total_interval / metrics_lock.interval_count as f64;
                self.context
                    .metrics
                    .subscriber_average_inter_block_time_seconds
                    .with_label_values(&[&self.id.to_string()])
                    .set(avg);
            }
        }
        self.send_final_status(final_code).await;
    }

    async fn send_final_status(&self, code: Code) {
        if let Code::Unknown = code {
            return;
        }
        info!(session_id = %self.id, "SENDING FINAL STATUS: {:?}", code);
        let response = SubscribeStreamResponse {
            response: Some(ResponseType::Status(code.into())),
        };
        if self.response_tx.send(Ok(response)).await.is_err() {
            debug!(session_id = %self.id, "Client disconnected before final status could be sent.");
        }
    }
}

struct DropGuard {
    gauge: prometheus::IntGauge,
}

#[derive(Default)]
struct InterBlockMetrics {
    last_time: Option<Instant>,
    total_interval: f64,
    interval_count: u64,
}

impl DropGuard {
    fn new(gauge: prometheus::IntGauge) -> Self {
        Self { gauge }
    }
}
impl Drop for DropGuard {
    fn drop(&mut self) {
        self.gauge.dec();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rock_node_core::{
        config::{
            BackfillConfig, BackfillMode, BlockAccessServiceConfig, Config, CoreConfig,
            ObservabilityConfig, PersistenceServiceConfig, PluginConfigs, PublishServiceConfig,
            QueryServiceConfig, ServerStatusServiceConfig, StateManagementServiceConfig,
            SubscriberServiceConfig, VerificationServiceConfig,
        },
        events::BlockPersisted,
        test_utils::create_isolated_metrics,
        AppContext, BlockReaderProvider, CapabilityRegistry,
    };
    use rock_node_protobufs::com::hedera::hapi::block::stream::BlockItem;
    use std::{any::Any, collections::HashMap, sync::RwLock};
    use tokio::sync::broadcast;

    #[derive(Debug, Default)]
    struct MockBlockReader {
        blocks: RwLock<HashMap<u64, Vec<u8>>>,
        earliest: RwLock<Option<u64>>,
        latest: RwLock<Option<u64>>,
    }

    impl BlockReader for MockBlockReader {
        fn get_latest_persisted_block_number(&self) -> anyhow::Result<Option<u64>> {
            Ok(*self.latest.read().unwrap())
        }
        fn get_earliest_persisted_block_number(&self) -> anyhow::Result<Option<u64>> {
            Ok(*self.earliest.read().unwrap())
        }
        fn read_block(&self, block_number: u64) -> anyhow::Result<Option<Vec<u8>>> {
            Ok(self.blocks.read().unwrap().get(&block_number).cloned())
        }
        fn get_highest_contiguous_block_number(&self) -> anyhow::Result<u64> {
            Ok(self.latest.read().unwrap().unwrap_or(0))
        }
    }

    impl MockBlockReader {
        fn add_block(&self, num: u64) {
            let mut earliest = self.earliest.write().unwrap();
            if earliest.is_none() {
                *earliest = Some(num);
            }
            *self.latest.write().unwrap() = Some(num);

            let block = Block {
                items: vec![BlockItem { item: None }],
            };
            let mut bytes = Vec::new();
            block.encode(&mut bytes).unwrap();
            self.blocks.write().unwrap().insert(num, bytes);
        }
    }

    fn create_test_context(
        reader: Arc<dyn BlockReader>,
    ) -> (Arc<AppContext>, broadcast::Sender<BlockPersisted>) {
        let config = Config {
            core: CoreConfig {
                log_level: "debug".to_string(),
                database_path: "".to_string(),
                start_block_number: 0,
                grpc_address: "".to_string(),
                grpc_port: 0,
            },
            plugins: PluginConfigs {
                observability: ObservabilityConfig {
                    enabled: false,
                    listen_address: "".to_string(),
                },
                persistence_service: PersistenceServiceConfig {
                    enabled: false,
                    cold_storage_path: "".to_string(),
                    hot_storage_block_count: 0,
                    archive_batch_size: 0,
                },
                publish_service: PublishServiceConfig {
                    enabled: false,
                    max_concurrent_streams: 0,
                    persistence_ack_timeout_seconds: 0,
                    stale_winner_timeout_seconds: 0,
                    winner_cleanup_interval_seconds: 0,
                    winner_cleanup_threshold_blocks: 0,
                },
                verification_service: VerificationServiceConfig { enabled: false },
                block_access_service: BlockAccessServiceConfig { enabled: false },
                server_status_service: ServerStatusServiceConfig { enabled: false },
                state_management_service: StateManagementServiceConfig { enabled: false },
                subscriber_service: SubscriberServiceConfig {
                    enabled: true,
                    max_concurrent_streams: 10,
                    session_timeout_seconds: 0,
                    live_stream_queue_size: 10,
                    max_future_block_lookahead: 100,
                },
                query_service: QueryServiceConfig { enabled: false },
                backfill: BackfillConfig {
                    enabled: false,
                    peers: vec![],
                    mode: BackfillMode::GapFill,
                    check_interval_seconds: 60,
                    max_batch_size: 1000,
                },
            },
        };

        let service_providers = Arc::new(RwLock::new(HashMap::new()));
        let provider = Arc::new(BlockReaderProvider::new(reader));
        service_providers.write().unwrap().insert(
            TypeId::of::<BlockReaderProvider>(),
            provider as Arc<dyn Any + Send + Sync>,
        );

        let (tx_persisted, _) = broadcast::channel(16);

        (
            Arc::new(AppContext {
                config: Arc::new(config),
                metrics: Arc::new(create_isolated_metrics()),
                capability_registry: Arc::new(CapabilityRegistry::new()),
                service_providers,
                block_data_cache: Arc::new(Default::default()),
                tx_block_items_received: tokio::sync::mpsc::channel(1).0,
                tx_block_verified: tokio::sync::mpsc::channel(1).0,
                tx_block_verification_failed: tokio::sync::broadcast::channel(1).0,
                tx_block_persisted: tx_persisted.clone(),
            }),
            tx_persisted,
        )
    }

    #[tokio::test]
    async fn test_finite_historical_stream() {
        let reader = MockBlockReader::default();
        for i in 0..=20 {
            reader.add_block(i);
        }
        let context = create_test_context(Arc::new(reader)).0;

        let request = SubscribeStreamRequest {
            start_block_number: 5,
            end_block_number: 10,
        };
        let (tx, mut rx) = mpsc::channel(16);

        // CORRECTED: Added missing Arc<Notify> arguments
        let server_shutdown = Arc::new(Notify::new());
        let session_shutdown = Arc::new(Notify::new());
        let mut session =
            SubscriberSession::new(context, request, tx, server_shutdown, session_shutdown)
                .expect("Failed to create test session");

        tokio::spawn(async move {
            session.run().await;
        });

        let mut block_count = 0;
        for _ in 5..=10 {
            let msg = rx.recv().await.unwrap().unwrap();
            if let Some(ResponseType::BlockItems(_)) = msg.response {
                block_count += 1;
            }
        }

        let final_msg = rx.recv().await.unwrap().unwrap();
        assert_eq!(block_count, 6);
        assert_eq!(
            final_msg.response,
            Some(ResponseType::Status(Code::Success.into()))
        );
    }

    #[tokio::test]
    async fn test_historical_to_live_to_finite_end() {
        let reader = Arc::new(MockBlockReader::default());
        for i in 0..=10 {
            reader.add_block(i);
        }
        let (context, tx_persisted) = create_test_context(reader.clone());

        let request = SubscribeStreamRequest {
            start_block_number: 5,
            end_block_number: 15,
        };
        let (tx, mut rx) = mpsc::channel(32);

        // CORRECTED: Added missing Arc<Notify> arguments
        let server_shutdown = Arc::new(Notify::new());
        let session_shutdown = Arc::new(Notify::new());
        let mut session =
            SubscriberSession::new(context, request, tx, server_shutdown, session_shutdown)
                .expect("Failed to create test session");

        tokio::spawn(async move {
            session.run().await;
        });

        for i in 5..=10 {
            let msg = rx.recv().await.unwrap().unwrap();
            assert!(
                matches!(msg.response, Some(ResponseType::BlockItems(_))),
                "Expected block {}, got {:?}",
                i,
                msg
            );
        }

        for i in 11..=15 {
            reader.add_block(i);
            tx_persisted
                .send(BlockPersisted {
                    block_number: i,
                    cache_key: Uuid::new_v4(),
                })
                .unwrap();
            let msg = rx.recv().await.unwrap().unwrap();
            assert!(
                matches!(msg.response, Some(ResponseType::BlockItems(_))),
                "Expected block {}, got {:?}",
                i,
                msg
            );
        }

        let final_msg = rx.recv().await.unwrap().unwrap();
        assert_eq!(
            final_msg.response,
            Some(ResponseType::Status(Code::Success.into()))
        );
    }

    #[tokio::test]
    async fn test_invalid_range_start_after_end() {
        let reader = Arc::new(MockBlockReader::default());
        let context = create_test_context(reader).0;
        let request = SubscribeStreamRequest {
            start_block_number: 10,
            end_block_number: 5,
        };
        let (tx, mut rx) = mpsc::channel(16);

        // CORRECTED: Added missing Arc<Notify> arguments
        let server_shutdown = Arc::new(Notify::new());
        let session_shutdown = Arc::new(Notify::new());
        let mut session =
            SubscriberSession::new(context, request, tx, server_shutdown, session_shutdown)
                .expect("Failed to create test session");

        tokio::spawn(async move {
            session.run().await;
        });

        let final_msg = rx.recv().await.unwrap().unwrap();
        assert_eq!(
            final_msg.response,
            Some(ResponseType::Status(Code::InvalidEndBlockNumber.into()))
        );
    }

    #[tokio::test]
    async fn test_stream_terminates_on_shutdown_signal() {
        let reader = Arc::new(MockBlockReader::default());
        reader.add_block(0);
        reader.add_block(1);
        let (context, _) = create_test_context(reader.clone());

        let request = SubscribeStreamRequest {
            start_block_number: 0,
            end_block_number: u64::MAX, // Infinite stream
        };
        let (tx, mut rx) = mpsc::channel(16);

        let server_shutdown = Arc::new(Notify::new());
        let session_shutdown = Arc::new(Notify::new());
        let mut session = SubscriberSession::new(
            context,
            request,
            tx,
            server_shutdown.clone(),
            session_shutdown,
        )
        .expect("Failed to create test session");

        tokio::spawn(async move {
            session.run().await;
        });

        // Receive the first two blocks
        assert!(matches!(
            rx.recv().await.unwrap().unwrap().response,
            Some(ResponseType::BlockItems(_))
        ));
        assert!(matches!(
            rx.recv().await.unwrap().unwrap().response,
            Some(ResponseType::BlockItems(_))
        ));

        // Now, signal the server to shut down
        server_shutdown.notify_one();

        // The session should send a final status and terminate
        let final_msg = rx.recv().await.unwrap().unwrap();
        assert_eq!(
            final_msg.response,
            Some(ResponseType::Status(Code::Error.into()))
        );

        // The channel should now be closed
        assert!(rx.recv().await.is_none());
    }
}
