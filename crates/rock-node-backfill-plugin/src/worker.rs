use anyhow::{anyhow, Context, Result};
use futures_util::Stream;
use rand::rng;
use rand::seq::SliceRandom;
use rock_node_core::{
    app_context::AppContext, block_reader::BlockReader, block_writer::BlockWriter,
    database::CF_GAPS, BlockReaderProvider, BlockWriterProvider,
};
use rock_node_protobufs::{
    com::hedera::hapi::block::stream::Block,
    org::hiero::block::api::{
        block_stream_subscribe_service_client::BlockStreamSubscribeServiceClient,
        subscribe_stream_response::Response as SubResponse, SubscribeStreamRequest,
    },
};
use rocksdb::IteratorMode;
use std::{any::TypeId, sync::Arc, time::Duration};
use tokio::sync::Notify;
use tokio_stream::{wrappers::ReceiverStream, StreamExt};
use tonic::transport::Channel;
use tracing::{error, info, trace, warn};

/// Establishes a connection to one of the provided peers and returns a stream of blocks.
///
/// This function is designed for external use. It allows any application to leverage
/// the peer-connection and block-streaming logic of this crate without being tied
/// to its database writing implementation.
///
/// # Arguments
/// * `peers` - A slice of strings, where each string is a valid URI for a peer's subscriber service.
/// * `start_block` - The block number to start streaming from.
/// * `end_block` - The block number to end the stream at. Use `u64::MAX` for a continuous stream.
///
/// # Returns
/// A `Result` containing a `Stream` of `Block`s. The outer `Result` will be an error
/// if a connection to any peer cannot be established. The inner `Result` within the stream
/// represents potential errors during the streaming process itself.
pub async fn stream_blocks_from_peers(
    peers: &[String],
    start_block: u64,
    end_block: u64,
) -> Result<impl Stream<Item = Result<Block, tonic::Status>>> {
    let mut shuffled_peers = peers.to_vec();
    shuffled_peers.shuffle(&mut rng());

    for peer_addr in &shuffled_peers {
        info!(
            "Attempting to connect to peer {} for blocks [{}, {}]",
            peer_addr, start_block, end_block
        );
        match Channel::from_shared(peer_addr.clone())?.connect().await {
            Ok(channel) => {
                let mut client = BlockStreamSubscribeServiceClient::new(channel);
                let request = SubscribeStreamRequest {
                    start_block_number: start_block,
                    end_block_number: end_block,
                };
                let stream = client.subscribe_block_stream(request).await?.into_inner();

                // We use a channel to map the stream items, which makes the return type cleaner.
                let (tx, rx) = tokio::sync::mpsc::channel(128);

                tokio::spawn(async move {
                    let mut stream = stream;
                    while let Some(msg_res) = stream.next().await {
                        match msg_res {
                            Ok(msg) => {
                                if let Some(SubResponse::BlockItems(item_set)) = msg.response {
                                    let block = Block {
                                        items: item_set.block_items,
                                    };
                                    if tx.send(Ok(block)).await.is_err() {
                                        // Receiver was dropped, so we can stop.
                                        break;
                                    }
                                }
                            }
                            Err(status) => {
                                // Forward the error through the channel and stop.
                                let _ = tx.send(Err(status)).await;
                                break;
                            }
                        }
                    }
                });

                // Return a stream that reads from the channel.
                return Ok(ReceiverStream::new(rx));
            }
            Err(e) => {
                warn!(
                    "Failed to connect to peer {}: {}. Trying next peer.",
                    peer_addr, e
                );
                continue;
            }
        }
    }
    Err(anyhow!("Failed to connect to any of the configured peers."))
}

#[derive(Debug)]
pub struct BackfillWorker {
    context: AppContext,
    block_reader: Arc<dyn BlockReader>,
    block_writer: Arc<dyn BlockWriter>,
    db: Arc<rocksdb::DB>,
}

impl BackfillWorker {
    pub fn new(context: AppContext) -> Result<Self> {
        let providers = context
            .service_providers
            .read()
            .map_err(|_| anyhow!("Service provider lock is poisoned"))?;

        let block_reader = providers
            .get(&TypeId::of::<BlockReaderProvider>())
            .and_then(|p| p.downcast_ref::<BlockReaderProvider>())
            .context("BackfillPlugin requires BlockReaderProvider")?
            .get_reader();

        let block_writer = providers
            .get(&TypeId::of::<BlockWriterProvider>())
            .and_then(|p| p.downcast_ref::<BlockWriterProvider>())
            .context("BackfillPlugin requires BlockWriterProvider")?
            .get_writer();

        let db_manager = providers
            .get(&TypeId::of::<
                rock_node_core::database_provider::DatabaseManagerProvider,
            >())
            .and_then(|p| {
                p.downcast_ref::<rock_node_core::database_provider::DatabaseManagerProvider>()
            })
            .context("BackfillPlugin requires DatabaseManagerProvider")?
            .get_manager();

        drop(providers);

        Ok(Self {
            context,
            block_reader,
            block_writer,
            db: db_manager.db_handle(),
        })
    }

    pub async fn run_gap_fill_loop(self: Arc<Self>, shutdown_notify: Arc<Notify>) {
        let interval_secs = self.context.config.plugins.backfill.check_interval_seconds;
        let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));

        loop {
            tokio::select! {
                _ = shutdown_notify.notified() => {
                    info!("GapFill loop received shutdown signal.");
                    break;
                }
                _ = interval.tick() => {
                    if let Err(e) = self.run_single_gap_check().await {
                        error!("Error during gap check cycle: {}", e);
                    }
                }
            }
        }
    }

    pub async fn run_continuous_loop(self: Arc<Self>, shutdown_notify: Arc<Notify>) {
        loop {
            let start_from = match self.block_reader.get_latest_persisted_block_number() {
                Ok(Some(num)) => num + 1,
                Ok(None) => self.context.config.core.start_block_number,
                Err(e) => {
                    error!(
                        "Could not read latest persisted block from DB: {}. Retrying...",
                        e
                    );
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
            };

            info!(
                "Starting continuous backfill stream from block #{}",
                start_from
            );

            let stream_future = self.establish_and_run_stream(start_from, u64::MAX);

            tokio::select! {
                res = stream_future => {
                    match res {
                        Ok(_) => info!("Continuous stream ended gracefully. Re-establishing connection."),
                        Err(e) => {
                            error!("Continuous stream failed: {}. Reconnecting after a delay.", e);
                            tokio::time::sleep(Duration::from_secs(5)).await;
                        }
                    }
                },
                _ = shutdown_notify.notified() => {
                    info!("Continuous loop received shutdown signal.");
                    break;
                }
            }
        }
    }

    async fn run_single_gap_check(&self) -> Result<()> {
        let gaps = self.get_all_gaps()?;
        if gaps.is_empty() {
            trace!("No gaps found to fill.");
            return Ok(());
        }

        info!("Found {} gaps to potentially fill.", gaps.len());
        for (start, end) in gaps {
            let effective_start = std::cmp::max(start, self.context.config.core.start_block_number);
            if effective_start > end {
                continue;
            }

            info!("Attempting to fill gap [{}, {}]", effective_start, end);
            if let Err(e) = self.establish_and_run_stream(effective_start, end).await {
                warn!(
                    "Failed to fill gap [{}, {}]: {}. Will retry on next cycle.",
                    effective_start, end, e
                );
            } else {
                info!("Successfully filled gap [{}, {}].", effective_start, end);
            }
        }
        Ok(())
    }

    /// (Refactored) Establishes a stream and writes incoming blocks to the database.
    async fn establish_and_run_stream(&self, start: u64, end: u64) -> Result<()> {
        let peers = &self.context.config.plugins.backfill.peers;
        let mut stream = stream_blocks_from_peers(peers, start, end).await?;

        while let Some(block_result) = stream.next().await {
            match block_result {
                Ok(block) => self.block_writer.write_block(&block).await?,
                Err(status) => return Err(anyhow!("gRPC stream error: {}", status)),
            }
        }
        Ok(())
    }

    fn get_all_gaps(&self) -> Result<Vec<(u64, u64)>> {
        let cf_gaps = self.db.cf_handle(CF_GAPS).context("CF_GAPS not found")?;
        let iter = self.db.iterator_cf(cf_gaps, IteratorMode::Start);
        let mut gaps = Vec::new();
        for item in iter {
            let (key_bytes, val_bytes) = item?;
            let start = u64::from_be_bytes(key_bytes.as_ref().try_into()?);
            let end = u64::from_be_bytes(val_bytes.as_ref().try_into()?);
            gaps.push((start, end));
        }
        Ok(gaps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use rock_node_core::{
        config::{BackfillConfig, Config, CoreConfig, PluginConfigs},
        database::DatabaseManager,
        database_provider::DatabaseManagerProvider,
        metrics::MetricsRegistry,
    };
    use rock_node_protobufs::{
        com::hedera::hapi::block::stream::{block_item, BlockItem},
        org::hiero::block::api::{
            block_stream_subscribe_service_server::{
                BlockStreamSubscribeService, BlockStreamSubscribeServiceServer,
            },
            subscribe_stream_response::Response as SubResponse,
            BlockItemSet, SubscribeStreamResponse,
        },
    };
    use std::collections::HashMap;
    use tempfile::TempDir;
    use tokio::net::TcpListener;
    use tokio_stream::wrappers::TcpListenerStream;
    use tonic::transport::Server;

    // --- Mock gRPC Peer Server ---
    #[derive(Default)]
    struct MockPeerServer;

    #[async_trait]
    impl BlockStreamSubscribeService for MockPeerServer {
        type subscribeBlockStreamStream =
            tokio_stream::wrappers::ReceiverStream<Result<SubscribeStreamResponse, tonic::Status>>;

        async fn subscribe_block_stream(
            &self,
            request: tonic::Request<SubscribeStreamRequest>,
        ) -> Result<tonic::Response<Self::subscribeBlockStreamStream>, tonic::Status> {
            let req = request.into_inner();
            let (tx, rx) = tokio::sync::mpsc::channel(10);

            tokio::spawn(async move {
                for i in req.start_block_number..=req.end_block_number {
                    let block = Block {
                        items: vec![BlockItem {
                            item: Some(block_item::Item::BlockHeader(
                                rock_node_protobufs::com::hedera::hapi::block::stream::output::BlockHeader {
                                    number: i, ..Default::default()
                                },
                            )),
                        }],
                    };
                    let item_set = BlockItemSet {
                        block_items: block.items,
                    };
                    let response = SubscribeStreamResponse {
                        response: Some(SubResponse::BlockItems(item_set)),
                    };
                    if tx.send(Ok(response)).await.is_err() {
                        break;
                    }
                }
            });

            Ok(tonic::Response::new(ReceiverStream::new(rx)))
        }
    }

    /// Helper to spawn a mock server and return its address.
    async fn spawn_mock_peer() -> Result<String> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let listener_stream = TcpListenerStream::new(listener);

        tokio::spawn(async move {
            Server::builder()
                .add_service(BlockStreamSubscribeServiceServer::new(
                    MockPeerServer::default(),
                ))
                .serve_with_incoming(listener_stream)
                .await
        });

        Ok(format!("http://{}", addr))
    }

    // --- NEW TESTS FOR stream_blocks_from_peers ---

    #[tokio::test]
    async fn test_stream_blocks_from_peers_success() {
        let peer_addr = spawn_mock_peer().await.unwrap();
        let peers = vec![peer_addr];

        let mut stream = stream_blocks_from_peers(&peers, 5, 10).await.unwrap();

        let mut received_blocks = Vec::new();
        while let Some(Ok(block)) = stream.next().await {
            let num = block.items[0].item.as_ref().unwrap().clone();
            if let block_item::Item::BlockHeader(h) = num {
                received_blocks.push(h.number);
            }
        }

        assert_eq!(received_blocks, vec![5, 6, 7, 8, 9, 10]);
    }

    #[tokio::test]
    async fn test_stream_blocks_failover() {
        let good_peer = spawn_mock_peer().await.unwrap();
        let peers = vec![
            "http://127.0.0.1:1".to_string(), // A bad peer that will fail to connect
            good_peer,
        ];

        let mut stream = stream_blocks_from_peers(&peers, 1, 3).await.unwrap();
        let mut received_blocks = Vec::new();
        while let Some(Ok(block)) = stream.next().await {
            let num = block.items[0].item.as_ref().unwrap().clone();
            if let block_item::Item::BlockHeader(h) = num {
                received_blocks.push(h.number);
            }
        }
        assert_eq!(received_blocks, vec![1, 2, 3]);
    }

    // --- Mock Implementations for Worker Tests ---

    #[derive(Debug)]
    struct MockBlockReader;
    #[async_trait]
    impl BlockReader for MockBlockReader {
        fn get_latest_persisted_block_number(&self) -> Result<Option<u64>> {
            Ok(Some(100))
        }
        fn read_block(&self, _block_number: u64) -> Result<Option<Vec<u8>>> {
            Ok(None)
        }
        fn get_earliest_persisted_block_number(&self) -> Result<Option<u64>> {
            Ok(None)
        }
        fn get_highest_contiguous_block_number(&self) -> Result<u64> {
            Ok(100)
        }
    }

    #[derive(Debug, Default)]
    struct MockBlockWriter {
        written_blocks: Arc<tokio::sync::Mutex<Vec<u64>>>,
    }
    impl MockBlockWriter {
        fn new() -> Self {
            Self {
                written_blocks: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            }
        }
    }
    #[async_trait]
    impl BlockWriter for MockBlockWriter {
        async fn write_block(&self, block: &Block) -> Result<()> {
            if let Some(rock_node_protobufs::com::hedera::hapi::block::stream::block_item::Item::BlockHeader(header)) = &block.items[0].item {
                self.written_blocks.lock().await.push(header.number);
            }
            Ok(())
        }
        async fn write_block_batch(&self, _blocks: &[Block]) -> Result<()> {
            Ok(())
        }
    }

    fn setup_test_context(
        temp_dir: &TempDir,
        peers: Vec<String>,
    ) -> (AppContext, Arc<MockBlockWriter>) {
        let db_manager = DatabaseManager::new(temp_dir.path().to_str().unwrap()).unwrap();

        let config = Config {
            plugins: PluginConfigs {
                backfill: BackfillConfig {
                    enabled: true,
                    peers,
                    ..Default::default()
                },
                ..Default::default()
            },
            core: CoreConfig {
                start_block_number: 0,
                ..Default::default()
            },
        };

        let mut providers: HashMap<TypeId, Arc<dyn std::any::Any + Send + Sync>> = HashMap::new();
        let writer = Arc::new(MockBlockWriter::new());

        providers.insert(
            TypeId::of::<BlockWriterProvider>(),
            Arc::new(BlockWriterProvider::new(writer.clone())),
        );
        providers.insert(
            TypeId::of::<DatabaseManagerProvider>(),
            Arc::new(DatabaseManagerProvider::new(Arc::new(db_manager))),
        );
        providers.insert(
            TypeId::of::<BlockReaderProvider>(),
            Arc::new(BlockReaderProvider::new(Arc::new(MockBlockReader))),
        );

        let context = AppContext {
            config: Arc::new(config),
            service_providers: Arc::new(std::sync::RwLock::new(providers)),
            metrics: Arc::new(MetricsRegistry::new().unwrap()),
            capability_registry: Arc::new(Default::default()),
            block_data_cache: Arc::new(Default::default()),
            tx_block_items_received: tokio::sync::mpsc::channel(100).0,
            tx_block_verified: tokio::sync::mpsc::channel(100).0,
            tx_block_persisted: tokio::sync::broadcast::channel(100).0,
        };

        (context, writer)
    }

    #[tokio::test]
    async fn test_worker_consumes_stream_and_writes() {
        let temp_dir = TempDir::new().unwrap();
        let peer_addr = spawn_mock_peer().await.unwrap();
        let (context, writer) = setup_test_context(&temp_dir, vec![peer_addr]);

        let worker = BackfillWorker::new(context).unwrap();

        // Run the worker's internal method
        worker.establish_and_run_stream(1, 5).await.unwrap();

        let written = writer.written_blocks.lock().await;
        assert_eq!(*written, vec![1, 2, 3, 4, 5]);
    }
}
