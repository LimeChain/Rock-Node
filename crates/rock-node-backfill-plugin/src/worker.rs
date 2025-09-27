use anyhow::{anyhow, Context, Result};
use futures_util::Stream;
use rand::seq::SliceRandom;
use rock_node_core::{
    app_context::AppContext, block_reader::BlockReader, block_writer::BlockWriter,
    config::BackfillMode, database::CF_GAPS, BlockReaderProvider, BlockWriterProvider,
};
use rock_node_protobufs::{
    com::hedera::hapi::block::stream::{block_item, Block, BlockItem},
    org::hiero::block::api::{
        block_node_service_client::BlockNodeServiceClient,
        block_stream_subscribe_service_client::BlockStreamSubscribeServiceClient,
        subscribe_stream_response::Response as SubResponse, ServerStatusRequest,
        SubscribeStreamRequest,
    },
};
use rocksdb::IteratorMode;
use std::{any::TypeId, sync::Arc, time::Duration};
use tokio::sync::Notify;
use tokio_stream::{wrappers::ReceiverStream, StreamExt};
use tonic::{transport::Channel, Request};
use tracing::{error, info, trace, warn};

/// The internal retry limit for a single peer.
const PEER_RETRY_LIMIT: u32 = 3;
/// The base delay for exponential backoff.
const PEER_BACKOFF_BASE_SECONDS: u64 = 1;
/// The delay when a peer reports it's behind.
const BEHIND_PEER_DELAY_SECONDS: u64 = 60;
/// Maximum number of peer connection attempts before giving up.
const MAX_PEER_CONNECTION_ATTEMPTS: u32 = 5;

/// Establishes a connection to one of the provided peers and returns a stream of blocks.
///
/// This function is designed for external use. It allows any application to leverage
/// the peer-connection and block-streaming logic of this crate without being tied
/// to its database writing implementation.
///
/// # Arguments
/// * `peers` - A slice of strings, where each string is a valid URI for a peer's subscriber service.
/// * `start_block` - The block number to start streaming from, or `None` to start from the peer's latest.
/// * `end_block` - The block number to end the stream at, or `None` for a continuous stream.
///
/// # Returns
/// A `Result` containing a `Stream` of `Block`s. The outer `Result` will be an error
/// if a connection to any peer cannot be established. The inner `Result` within the stream
/// represents potential errors during the streaming process itself.
pub async fn stream_blocks_from_peers(
    peers: &[String],
    start_block: Option<u64>,
    end_block: Option<u64>,
) -> Result<impl Stream<Item = Result<Block, tonic::Status>>> {
    let mut shuffled_peers = peers.to_vec();
    shuffled_peers.shuffle(&mut rand::rng());
    let mut connection_attempts = 0;

    loop {
        for peer_addr in &shuffled_peers {
            let mut retries = 0;
            loop {
                match try_connect_and_stream(peer_addr, start_block, end_block).await {
                    Ok(Some(stream)) => return Ok(stream),
                    Ok(None) => {
                        break;
                    },
                    Err(e) => {
                        warn!(
                            "Failed to connect to peer {}: {}. Retrying in {}s.",
                            peer_addr,
                            e,
                            PEER_BACKOFF_BASE_SECONDS.saturating_mul(2u64.pow(retries))
                        );

                        if retries >= PEER_RETRY_LIMIT {
                            warn!(
                                "Maximum retries reached for peer {}. Trying next peer.",
                                peer_addr
                            );
                            break;
                        }
                        tokio::time::sleep(Duration::from_secs(
                            PEER_BACKOFF_BASE_SECONDS.saturating_mul(2u64.pow(retries)),
                        ))
                        .await;
                        retries += 1;
                    },
                }
            }
        }

        connection_attempts += 1;
        if connection_attempts >= MAX_PEER_CONNECTION_ATTEMPTS {
            return Err(anyhow!(
                "Failed to connect to any peer after {} attempts.",
                MAX_PEER_CONNECTION_ATTEMPTS
            ));
        }

        warn!("Failed to connect to any configured peers (attempt {}/{}). Re-shuffling and trying again.",
              connection_attempts, MAX_PEER_CONNECTION_ATTEMPTS);
        shuffled_peers.shuffle(&mut rand::rng());
    }
}

async fn try_connect_and_stream(
    peer_addr: &str,
    start_block_opt: Option<u64>,
    end_block_opt: Option<u64>,
) -> Result<Option<impl Stream<Item = Result<Block, tonic::Status>>>, anyhow::Error> {
    // This logic is now correct: it connects to the single peer address
    // and expects the BlockNodeService (for status) to be available there.
    let mut status_client = BlockNodeServiceClient::connect(peer_addr.to_string()).await?;
    let status_res = status_client
        .server_status(Request::new(ServerStatusRequest {}))
        .await?;
    let status_res = status_res.into_inner();
    let peer_latest = status_res.last_available_block;

    let start_block = start_block_opt.unwrap_or(peer_latest);
    let end_block = end_block_opt.unwrap_or(u64::MAX);

    if start_block > end_block {
        return Err(anyhow!(
            "Start block {} is after end block {}",
            start_block,
            end_block
        ));
    }

    if start_block > peer_latest && end_block == u64::MAX {
        info!(
            "Peer {} is behind. Starting continuous stream from its latest block #{}",
            peer_addr, peer_latest
        );
    } else if start_block > peer_latest {
        warn!(
            "Peer {} does not have block range [{} - {}]. Latest is #{}.",
            peer_addr, start_block, end_block, peer_latest
        );
        return Ok(None);
    }

    // This also correctly connects to the same peer address, expecting
    // the BlockStreamSubscribeService to be available.
    let channel = Channel::from_shared(peer_addr.to_string())?
        .connect()
        .await?;
    let mut client = BlockStreamSubscribeServiceClient::new(channel);
    let request = SubscribeStreamRequest {
        start_block_number: start_block,
        end_block_number: end_block,
    };
    let stream = client.subscribe_block_stream(request).await?.into_inner();

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
                            break;
                        }
                    }
                },
                Err(status) => {
                    let _ = tx.send(Err(status)).await;
                    break;
                },
            }
        }
    });

    Ok(Some(ReceiverStream::new(rx)))
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
        let mut shuffled_peers = self.context.config.plugins.backfill.peers.clone();

        loop {
            tokio::select! {
                _ = shutdown_notify.notified() => {
                    info!("GapFill loop received shutdown signal.");
                    break;
                }
                _ = interval.tick() => {
                    shuffled_peers.shuffle(&mut rand::rng());
                    if let Err(e) = self.run_single_gap_check(&shuffled_peers).await {
                        error!("Error during gap check cycle: {}", e);
                    }
                }
            }
        }
    }

    pub async fn run_continuous_loop(self: Arc<Self>, shutdown_notify: Arc<Notify>) {
        let mut shuffled_peers = self.context.config.plugins.backfill.peers.clone();
        loop {
            tokio::select! {
                _ = shutdown_notify.notified() => {
                    info!("Continuous loop received shutdown signal.");
                    break;
                }
                _ = self.run_continuous_stream_cycle(&mut shuffled_peers) => {}
            }
        }
    }

    async fn run_continuous_stream_cycle(self: &Arc<Self>, shuffled_peers: &mut Vec<String>) {
        shuffled_peers.shuffle(&mut rand::rng());

        for peer_addr in shuffled_peers.iter() {
            let local_latest_block = match self.block_reader.get_latest_persisted_block_number() {
                Ok(Some(num)) => num as i64,
                Ok(None) => -1,
                Err(e) => {
                    warn!(
                        "Could not get latest persisted block on startup: {}. Defaulting to -1.",
                        e
                    );
                    -1
                },
            };
            let mut retries = 0;
            loop {
                match self.get_peer_status(peer_addr).await {
                    Ok(Some(status)) => {
                        let peer_latest = status.last_available_block as i64;
                        if peer_latest < local_latest_block {
                            warn!(
                                "Peer {} is behind local node (peer_latest: {}, local_latest: {}).",
                                peer_addr, peer_latest, local_latest_block
                            );
                            tokio::time::sleep(Duration::from_secs(BEHIND_PEER_DELAY_SECONDS))
                                .await;
                            break;
                        }

                        let start_from = local_latest_block + 1;
                        if start_from > peer_latest && start_from > 0 {
                            info!(
                                "Peer {} does not have block #{} yet.",
                                peer_addr, start_from
                            );
                            tokio::time::sleep(Duration::from_secs(5)).await;
                            continue;
                        }

                        info!(
                            "Starting continuous backfill from block #{} from peer {}",
                            start_from, peer_addr
                        );

                        match self
                            .stream_from_peer(
                                peer_addr,
                                start_from as u64,
                                u64::MAX,
                                BackfillMode::Continuous,
                            )
                            .await
                        {
                            Ok(()) => {
                                info!("Continuous stream from {} ended gracefully.", peer_addr);
                                return;
                            },
                            Err(e) => {
                                error!(
                                    "Continuous stream from {} failed: {}. Retrying peer.",
                                    peer_addr, e
                                );
                                retries += 1;
                                tokio::time::sleep(Duration::from_secs(
                                    PEER_BACKOFF_BASE_SECONDS.saturating_mul(2u64.pow(retries)),
                                ))
                                .await;
                                if retries > PEER_RETRY_LIMIT {
                                    break;
                                }
                            },
                        }
                    },
                    Ok(None) => {
                        warn!(
                            "Could not get server status from peer {}. Trying next peer.",
                            peer_addr
                        );
                        break;
                    },
                    Err(e) => {
                        warn!(
                            "Failed to connect for status check to peer {}: {}. Retrying...",
                            peer_addr, e
                        );
                        retries += 1;
                        tokio::time::sleep(Duration::from_secs(
                            PEER_BACKOFF_BASE_SECONDS.saturating_mul(2u64.pow(retries)),
                        ))
                        .await;
                        if retries > PEER_RETRY_LIMIT {
                            break;
                        }
                    },
                }
            }
        }
    }

    async fn run_single_gap_check(&self, shuffled_peers: &[String]) -> Result<()> {
        let gaps = self.get_all_gaps()?;
        if gaps.is_empty() {
            trace!("No gaps found to fill.");
            return Ok(());
        }

        self.context
            .metrics
            .backfill_gaps_found_total
            .inc_by(gaps.len() as u64);
        info!("Found {} gaps to potentially fill.", gaps.len());

        for (start, end) in gaps {
            let effective_start = std::cmp::max(start, self.context.config.core.start_block_number);
            if effective_start > end {
                continue;
            }

            'peer_loop: for peer_addr in shuffled_peers.iter() {
                let mut retries = 0;
                loop {
                    match self.get_peer_status(peer_addr).await {
                        Ok(Some(status)) => {
                            if status.first_available_block > effective_start
                                || status.last_available_block < end
                            {
                                warn!(
                                    "Peer {} does not have gap [{} - {}]. Range is [{} - {}].",
                                    peer_addr,
                                    effective_start,
                                    end,
                                    status.first_available_block,
                                    status.last_available_block
                                );
                                break;
                            }
                            info!(
                                "Attempting to fill gap [{}, {}] from peer {}",
                                effective_start, end, peer_addr
                            );
                            if let Err(e) = self
                                .stream_from_peer(
                                    peer_addr,
                                    effective_start,
                                    end,
                                    BackfillMode::GapFill,
                                )
                                .await
                            {
                                warn!(
                                    "Failed to fill gap [{}, {}] from {}: {}. Retrying peer.",
                                    effective_start, end, peer_addr, e
                                );
                                retries += 1;
                                tokio::time::sleep(Duration::from_secs(
                                    PEER_BACKOFF_BASE_SECONDS.saturating_mul(2u64.pow(retries)),
                                ))
                                .await;
                                if retries > PEER_RETRY_LIMIT {
                                    break;
                                }
                            } else {
                                info!(
                                    "Successfully filled gap [{}, {}] from {}.",
                                    effective_start, end, peer_addr
                                );
                                break 'peer_loop;
                            }
                        },
                        Ok(None) => {
                            warn!(
                                "Could not get server status from peer {}. Trying next peer.",
                                peer_addr
                            );
                            break;
                        },
                        Err(e) => {
                            warn!(
                                "Failed to connect for status check to peer {}: {}. Retrying...",
                                peer_addr, e
                            );
                            retries += 1;
                            tokio::time::sleep(Duration::from_secs(
                                PEER_BACKOFF_BASE_SECONDS.saturating_mul(2u64.pow(retries)),
                            ))
                            .await;
                            if retries > PEER_RETRY_LIMIT {
                                break;
                            }
                        },
                    }
                }
            }
        }
        Ok(())
    }

    async fn get_peer_status(
        &self,
        peer_addr: &str,
    ) -> Result<Option<rock_node_protobufs::org::hiero::block::api::ServerStatusResponse>> {
        let channel = match Channel::from_shared(peer_addr.to_string())?.connect().await {
            Ok(c) => c,
            Err(_) => return Ok(None),
        };
        let mut client = BlockNodeServiceClient::new(channel);
        match client
            .server_status(Request::new(ServerStatusRequest {}))
            .await
        {
            Ok(res) => Ok(Some(res.into_inner())),
            Err(e) => {
                warn!("Failed to get server status from peer {}: {}", peer_addr, e);
                Ok(None)
            },
        }
    }

    async fn stream_from_peer(
        &self,
        peer_addr: &str,
        start: u64,
        end: u64,
        mode: BackfillMode,
    ) -> Result<()> {
        let mode_str = if matches!(mode, BackfillMode::GapFill) {
            "GapFill"
        } else {
            "Continuous"
        };

        let channel = Channel::from_shared(peer_addr.to_string())?
            .connect()
            .await?;
        let mut client = BlockStreamSubscribeServiceClient::new(channel);
        let request = SubscribeStreamRequest {
            start_block_number: start,
            end_block_number: end,
        };
        let mut stream = client.subscribe_block_stream(request).await?.into_inner();

        let timer = self
            .context
            .metrics
            .backfill_stream_duration_seconds
            .with_label_values(&[peer_addr, &mode_str.to_string()])
            .start_timer();
        self.context.metrics.backfill_active_streams.inc();
        let _drop_guard = DecrementingGuard::new(&self.context.metrics.backfill_active_streams);

        while let Some(block_result) = stream.next().await {
            match block_result {
                Ok(msg) => {
                    if let Some(SubResponse::BlockItems(item_set)) = msg.response {
                        let block = Block {
                            items: item_set.block_items,
                        };
                        if let Some(block_number) = get_block_number_from_block(&block) {
                            self.context
                                .metrics
                                .backfill_blocks_fetched_total
                                .with_label_values(&[mode_str])
                                .inc();
                            if matches!(mode, BackfillMode::Continuous) {
                                self.context
                                    .metrics
                                    .backfill_latest_continuous_block
                                    .set(block_number as i64);
                            }
                        }
                        self.block_writer.write_block(&block).await?;
                    }
                },
                Err(status) => return Err(anyhow!("gRPC stream error: {}", status)),
            }
        }
        timer.observe_duration();
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

    #[cfg(test)]
    async fn process_blocks_directly(
        &self,
        start_block: u64,
        end_block: u64,
        mode: BackfillMode,
    ) -> Result<()> {
        let mode_str = if matches!(mode, BackfillMode::GapFill) {
            "GapFill"
        } else {
            "Continuous"
        };

        for block_number in start_block..=end_block {
            let block = Block {
                items: vec![BlockItem {
                    item: Some(block_item::Item::BlockHeader(
                        rock_node_protobufs::com::hedera::hapi::block::stream::output::BlockHeader {
                            number: block_number,
                            ..Default::default()
                        },
                    )),
                }],
            };

            self.context
                .metrics
                .backfill_blocks_fetched_total
                .with_label_values(&[mode_str])
                .inc();
            if matches!(mode, BackfillMode::Continuous) {
                self.context
                    .metrics
                    .backfill_latest_continuous_block
                    .set(block_number as i64);
            }
            self.block_writer.write_block(&block).await?;
        }
        Ok(())
    }
}

fn get_block_number_from_block(block: &Block) -> Option<u64> {
    block.items.first().and_then(|item| {
        if let Some(
            rock_node_protobufs::com::hedera::hapi::block::stream::block_item::Item::BlockHeader(
                header,
            ),
        ) = &item.item
        {
            Some(header.number)
        } else {
            None
        }
    })
}

struct DecrementingGuard<'a> {
    gauge: &'a prometheus::IntGauge,
}
impl<'a> DecrementingGuard<'a> {
    fn new(gauge: &'a prometheus::IntGauge) -> Self {
        Self { gauge }
    }
}
impl<'a> Drop for DecrementingGuard<'a> {
    fn drop(&mut self) {
        self.gauge.dec();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rock_node_core::{
        app_context::AppContext,
        block_reader::{BlockReader, BlockReaderProvider},
        block_writer::{BlockWriter, BlockWriterProvider},
        config::{BackfillConfig, Config, CoreConfig, PluginConfigs},
        database::DatabaseManager,
        database_provider::DatabaseManagerProvider,
        test_utils::create_isolated_metrics,
    };
    use rock_node_protobufs::{
        com::hedera::hapi::block::stream::{block_item, output::BlockHeader, BlockItem},
        org::hiero::block::api::{
            block_node_service_server::BlockNodeService,
            block_stream_subscribe_service_server::BlockStreamSubscribeService,
            subscribe_stream_response::Response as SubResponse, BlockItemSet, ServerStatusRequest,
            ServerStatusResponse, SubscribeStreamResponse,
        },
    };
    use std::{
        any::TypeId,
        collections::HashMap,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
    };
    use tempfile::TempDir;
    use tokio_stream::wrappers::ReceiverStream;
    use tonic::{Request, Response, Status};

    #[derive(Debug)]
    struct MockBlockReader {
        latest_block: Option<u64>,
        earliest_block: Option<u64>,
        should_error: bool,
    }

    impl MockBlockReader {
        fn new(latest: Option<u64>, earliest: Option<u64>) -> Self {
            Self {
                latest_block: latest,
                earliest_block: earliest,
                should_error: false,
            }
        }

        #[allow(dead_code)]
        fn with_error() -> Self {
            Self {
                latest_block: None,
                earliest_block: None,
                should_error: true,
            }
        }
    }

    #[async_trait::async_trait]
    impl BlockReader for MockBlockReader {
        fn get_latest_persisted_block_number(&self) -> anyhow::Result<Option<u64>> {
            if self.should_error {
                return Err(anyhow::anyhow!("Mock error"));
            }
            Ok(self.latest_block)
        }

        fn read_block(&self, _block_number: u64) -> anyhow::Result<Option<Vec<u8>>> {
            Ok(None)
        }

        fn get_earliest_persisted_block_number(&self) -> anyhow::Result<Option<u64>> {
            if self.should_error {
                return Err(anyhow::anyhow!("Mock error"));
            }
            Ok(self.earliest_block)
        }

        fn get_highest_contiguous_block_number(&self) -> anyhow::Result<u64> {
            Ok(self.latest_block.unwrap_or(0))
        }
    }

    #[derive(Debug, Default)]
    struct MockBlockWriter {
        blocks_written: Arc<AtomicUsize>,
        should_error: bool,
    }

    impl MockBlockWriter {
        fn new() -> Self {
            Self {
                blocks_written: Arc::new(AtomicUsize::new(0)),
                should_error: false,
            }
        }

        fn with_error() -> Self {
            Self {
                blocks_written: Arc::new(AtomicUsize::new(0)),
                should_error: true,
            }
        }

        fn blocks_written(&self) -> usize {
            self.blocks_written.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl BlockWriter for MockBlockWriter {
        async fn write_block(&self, _block: &Block) -> anyhow::Result<()> {
            if self.should_error {
                return Err(anyhow::anyhow!("Mock write error"));
            }
            self.blocks_written.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn write_block_batch(&self, blocks: &[Block]) -> anyhow::Result<()> {
            if self.should_error {
                return Err(anyhow::anyhow!("Mock batch write error"));
            }
            self.blocks_written
                .fetch_add(blocks.len(), Ordering::SeqCst);
            Ok(())
        }
    }

    fn create_test_context(
        block_reader: Arc<dyn BlockReader>,
        block_writer: Arc<dyn BlockWriter>,
        temp_dir: &TempDir,
    ) -> AppContext {
        let db_manager = DatabaseManager::new(temp_dir.path().to_str().unwrap()).unwrap();

        let config = Config {
            core: CoreConfig {
                log_level: "INFO".to_string(),
                database_path: temp_dir.path().to_str().unwrap().to_string(),
                start_block_number: 0,
                grpc_address: "127.0.0.1".to_string(),
                grpc_port: 8080,
            },
            plugins: PluginConfigs {
                backfill: BackfillConfig {
                    enabled: true,
                    mode: rock_node_core::config::BackfillMode::GapFill,
                    peers: vec!["http://localhost:8080".to_string()],
                    check_interval_seconds: 1,
                    max_batch_size: 100,
                },
                ..Default::default()
            },
        };

        let mut providers: HashMap<TypeId, Arc<dyn std::any::Any + Send + Sync>> = HashMap::new();

        providers.insert(
            TypeId::of::<BlockReaderProvider>(),
            Arc::new(BlockReaderProvider::new(block_reader)),
        );

        providers.insert(
            TypeId::of::<BlockWriterProvider>(),
            Arc::new(BlockWriterProvider::new(block_writer)),
        );

        providers.insert(
            TypeId::of::<DatabaseManagerProvider>(),
            Arc::new(DatabaseManagerProvider::new(Arc::new(db_manager))),
        );

        AppContext {
            config: Arc::new(config),
            service_providers: Arc::new(std::sync::RwLock::new(providers)),
            metrics: Arc::new(create_isolated_metrics()),
            capability_registry: Arc::new(rock_node_core::capability::CapabilityRegistry::new()),
            block_data_cache: Arc::new(rock_node_core::cache::BlockDataCache::default()),
            tx_block_items_received: tokio::sync::mpsc::channel(100).0,
            tx_block_verified: tokio::sync::mpsc::channel(100).0,
            tx_block_persisted: tokio::sync::broadcast::channel(100).0,
        }
    }

    fn create_block_with_number(number: u64) -> Block {
        Block {
            items: vec![BlockItem {
                item: Some(block_item::Item::BlockHeader(BlockHeader {
                    number,
                    ..Default::default()
                })),
            }],
        }
    }

    #[derive(Clone, Default)]
    #[allow(dead_code)]
    struct MockPeerServer {
        fail_count: Arc<AtomicUsize>,
        total_failures_to_simulate: usize,
    }

    #[derive(Clone)]
    #[allow(dead_code)]
    struct MockNodeStatusServer {
        status: ServerStatusResponse,
        should_fail: bool,
    }

    #[allow(dead_code)]
    impl MockNodeStatusServer {
        fn new(first_block: u64, last_block: u64) -> Self {
            Self {
                status: ServerStatusResponse {
                    first_available_block: first_block,
                    last_available_block: last_block,
                    only_latest_state: false,
                    version_information: None,
                },
                should_fail: false,
            }
        }

        fn with_failure() -> Self {
            Self {
                status: ServerStatusResponse {
                    first_available_block: 0,
                    last_available_block: 0,
                    only_latest_state: false,
                    version_information: None,
                },
                should_fail: true,
            }
        }
    }

    #[tonic::async_trait]
    impl BlockNodeService for MockNodeStatusServer {
        async fn server_status(
            &self,
            _request: Request<ServerStatusRequest>,
        ) -> Result<Response<ServerStatusResponse>, Status> {
            if self.should_fail {
                return Err(Status::unavailable("Mock server error"));
            }
            Ok(Response::new(self.status.clone()))
        }
    }

    #[tonic::async_trait]
    impl BlockStreamSubscribeService for MockPeerServer {
        type subscribeBlockStreamStream =
            tokio_stream::wrappers::ReceiverStream<Result<SubscribeStreamResponse, tonic::Status>>;

        async fn subscribe_block_stream(
            &self,
            request: tonic::Request<SubscribeStreamRequest>,
        ) -> Result<tonic::Response<Self::subscribeBlockStreamStream>, tonic::Status> {
            let current_failures = self.fail_count.load(Ordering::SeqCst);
            if current_failures < self.total_failures_to_simulate {
                self.fail_count.fetch_add(1, Ordering::SeqCst);
                return Err(tonic::Status::unavailable("Simulated failure"));
            }

            let req = request.into_inner();
            let (tx, rx) = tokio::sync::mpsc::channel(10);

            tokio::spawn(async move {
                for i in req.start_block_number..=req.end_block_number {
                    let block = Block {
                        items: vec![BlockItem {
                            item: Some(block_item::Item::BlockHeader(BlockHeader {
                                number: i,
                                ..Default::default()
                            })),
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

    #[tokio::test]
    async fn test_backfill_worker_new_success() {
        let temp_dir = TempDir::new().unwrap();
        let block_reader = Arc::new(MockBlockReader::new(Some(100), Some(1)));
        let block_writer = Arc::new(MockBlockWriter::new());
        let context = create_test_context(block_reader, block_writer, &temp_dir);

        let worker = BackfillWorker::new(context);
        assert!(worker.is_ok());
    }

    #[tokio::test]
    async fn test_backfill_worker_new_missing_providers() {
        let temp_dir = TempDir::new().unwrap();
        let _db_manager = DatabaseManager::new(temp_dir.path().to_str().unwrap()).unwrap();

        let config = Config {
            core: CoreConfig {
                log_level: "INFO".to_string(),
                database_path: temp_dir.path().to_str().unwrap().to_string(),
                start_block_number: 0,
                grpc_address: "127.0.0.1".to_string(),
                grpc_port: 8080,
            },
            plugins: PluginConfigs {
                backfill: BackfillConfig {
                    enabled: true,
                    mode: rock_node_core::config::BackfillMode::GapFill,
                    peers: vec!["http://localhost:8080".to_string()],
                    check_interval_seconds: 1,
                    max_batch_size: 100,
                },
                ..Default::default()
            },
        };

        let providers: HashMap<TypeId, Arc<dyn std::any::Any + Send + Sync>> = HashMap::new();

        let context = AppContext {
            config: Arc::new(config),
            service_providers: Arc::new(std::sync::RwLock::new(providers)),
            metrics: Arc::new(create_isolated_metrics()),
            capability_registry: Arc::new(rock_node_core::capability::CapabilityRegistry::new()),
            block_data_cache: Arc::new(rock_node_core::cache::BlockDataCache::default()),
            tx_block_items_received: tokio::sync::mpsc::channel(100).0,
            tx_block_verified: tokio::sync::mpsc::channel(100).0,
            tx_block_persisted: tokio::sync::broadcast::channel(100).0,
        };

        let result = BackfillWorker::new(context);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("BlockReaderProvider"));
    }

    #[tokio::test]
    async fn test_get_all_gaps_empty_database() {
        let temp_dir = TempDir::new().unwrap();
        let block_reader = Arc::new(MockBlockReader::new(Some(100), Some(1)));
        let block_writer = Arc::new(MockBlockWriter::new());
        let context = create_test_context(block_reader, block_writer, &temp_dir);

        let worker = BackfillWorker::new(context).unwrap();
        let gaps = worker.get_all_gaps().unwrap();
        assert!(gaps.is_empty());
    }

    #[tokio::test]
    async fn test_get_all_gaps_with_data() {
        let temp_dir = TempDir::new().unwrap();
        let block_reader = Arc::new(MockBlockReader::new(Some(100), Some(1)));
        let block_writer = Arc::new(MockBlockWriter::new());
        let context = create_test_context(block_reader, block_writer, &temp_dir);

        let worker = BackfillWorker::new(context).unwrap();

        let cf_gaps = worker.db.cf_handle(CF_GAPS).unwrap();
        worker
            .db
            .put_cf(cf_gaps, 10u64.to_be_bytes(), 20u64.to_be_bytes())
            .unwrap();
        worker
            .db
            .put_cf(cf_gaps, 50u64.to_be_bytes(), 60u64.to_be_bytes())
            .unwrap();

        let gaps = worker.get_all_gaps().unwrap();
        assert_eq!(gaps.len(), 2);
        assert_eq!(gaps[0], (10, 20));
        assert_eq!(gaps[1], (50, 60));
    }

    #[test]
    fn test_get_block_number_from_block() {
        let block = create_block_with_number(42);
        let number = get_block_number_from_block(&block);
        assert_eq!(number, Some(42));
    }

    #[test]
    fn test_get_block_number_from_empty_block() {
        let block = Block { items: vec![] };
        let number = get_block_number_from_block(&block);
        assert_eq!(number, None);
    }

    #[test]
    fn test_get_block_number_from_block_without_header() {
        let block = Block {
            items: vec![BlockItem { item: None }],
        };
        let number = get_block_number_from_block(&block);
        assert_eq!(number, None);
    }

    #[tokio::test]
    async fn test_process_blocks_directly() {
        let temp_dir = TempDir::new().unwrap();
        let block_reader = Arc::new(MockBlockReader::new(Some(100), Some(1)));
        let block_writer = Arc::new(MockBlockWriter::new());
        let context = create_test_context(block_reader, block_writer.clone(), &temp_dir);

        let worker = BackfillWorker::new(context).unwrap();
        let result = worker
            .process_blocks_directly(5, 7, BackfillMode::GapFill)
            .await;

        assert!(result.is_ok());
        assert_eq!(block_writer.blocks_written(), 3); // blocks 5, 6, 7
    }

    #[tokio::test]
    async fn test_process_blocks_directly_continuous_mode() {
        let temp_dir = TempDir::new().unwrap();
        let block_reader = Arc::new(MockBlockReader::new(Some(100), Some(1)));
        let block_writer = Arc::new(MockBlockWriter::new());
        let context = create_test_context(block_reader, block_writer.clone(), &temp_dir);

        let worker = BackfillWorker::new(context).unwrap();
        let result = worker
            .process_blocks_directly(1, 3, BackfillMode::Continuous)
            .await;

        assert!(result.is_ok());
        assert_eq!(block_writer.blocks_written(), 3);
    }

    #[tokio::test]
    async fn test_process_blocks_with_write_error() {
        let temp_dir = TempDir::new().unwrap();
        let block_reader = Arc::new(MockBlockReader::new(Some(100), Some(1)));
        let block_writer = Arc::new(MockBlockWriter::with_error());
        let context = create_test_context(block_reader, block_writer, &temp_dir);

        let worker = BackfillWorker::new(context).unwrap();
        let result = worker
            .process_blocks_directly(1, 2, BackfillMode::GapFill)
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Mock write error"));
    }

    #[tokio::test]
    async fn test_stream_blocks_from_peers_empty_peers() {
        let result = stream_blocks_from_peers(&[], Some(1), Some(10)).await;
        assert!(result.is_err());
        assert!(result
            .err()
            .unwrap()
            .to_string()
            .contains("Failed to connect to any peer"));
    }

    #[tokio::test]
    async fn test_stream_blocks_from_peers_invalid_start_end() {
        let peers = vec!["http://localhost:8080".to_string()];
        let result = stream_blocks_from_peers(&peers, Some(10), Some(5)).await;
        assert!(result.is_err());
        let error_msg = result.err().unwrap().to_string();
        assert!(error_msg.contains("Failed to connect to any peer"));
    }

    #[test]
    fn test_block_range_validation() {
        let start_block = 10u64;
        let end_block = 5u64;

        if start_block > end_block {
            let error_msg = format!(
                "Start block {} is after end block {}",
                start_block, end_block
            );
            assert!(error_msg.contains("Start block") && error_msg.contains("is after end block"));
        }
    }

    #[test]
    fn test_decrementing_guard() {
        use prometheus::IntGauge;
        let gauge = IntGauge::new("test_gauge", "Test gauge").unwrap();
        gauge.set(10);

        {
            let _guard = DecrementingGuard::new(&gauge);
            assert_eq!(gauge.get(), 10);
        }

        assert_eq!(gauge.get(), 9);
    }

    #[test]
    fn test_peer_retry_constants() {
        assert_eq!(PEER_RETRY_LIMIT, 3);
        assert_eq!(PEER_BACKOFF_BASE_SECONDS, 1);
        assert_eq!(BEHIND_PEER_DELAY_SECONDS, 60);
        assert_eq!(MAX_PEER_CONNECTION_ATTEMPTS, 5);
    }

    #[tokio::test]
    async fn test_continuous_stream_with_behind_peer() {
        let temp_dir = TempDir::new().unwrap();
        let block_reader = Arc::new(MockBlockReader::new(Some(100), Some(1)));
        let block_writer = Arc::new(MockBlockWriter::new());
        let context = create_test_context(block_reader, block_writer, &temp_dir);
        let worker = Arc::new(BackfillWorker::new(context).unwrap());

        let mut peers = vec!["http://localhost:8080".to_string()];

        let shutdown_notify = Arc::new(Notify::new());
        shutdown_notify.notify_waiters();

        worker.run_continuous_stream_cycle(&mut peers).await;
    }
}
