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
                    }
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
                    }
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
                }
                Err(status) => {
                    let _ = tx.send(Err(status)).await;
                    break;
                }
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
            let local_latest_block = self
                .block_reader
                .get_latest_persisted_block_number()
                .ok()
                .flatten()
                .unwrap_or(0);
            let mut retries = 0;
            loop {
                match self.get_peer_status(peer_addr).await {
                    Ok(Some(status)) => {
                        let peer_latest = status.last_available_block;
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
                                start_from,
                                u64::MAX,
                                BackfillMode::Continuous,
                            )
                            .await
                        {
                            Ok(()) => {
                                info!("Continuous stream from {} ended gracefully.", peer_addr);
                                return;
                            }
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
                            }
                        }
                    }
                    Ok(None) => {
                        warn!(
                            "Could not get server status from peer {}. Trying next peer.",
                            peer_addr
                        );
                        break;
                    }
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
                    }
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
                        }
                        Ok(None) => {
                            warn!(
                                "Could not get server status from peer {}. Trying next peer.",
                                peer_addr
                            );
                            break;
                        }
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
                        }
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
            }
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
                }
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

    use rock_node_protobufs::{
        com::hedera::hapi::block::stream::{block_item, BlockItem},
        org::hiero::block::api::{
            block_stream_subscribe_service_server::BlockStreamSubscribeService,
            subscribe_stream_response::Response as SubResponse, BlockItemSet,
            SubscribeStreamResponse,
        },
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Clone, Default)]
    struct MockPeerServer {
        fail_count: Arc<AtomicUsize>,
        total_failures_to_simulate: usize,
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
}
