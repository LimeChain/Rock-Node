use anyhow::{anyhow, Context, Result};
use rock_node_core::{
    app_context::AppContext,
    block_reader::BlockReader,
    block_writer::BlockWriter,
    database::CF_GAPS,
    BlockReaderProvider, BlockWriterProvider,
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
use tokio_stream::StreamExt;
use tonic::transport::Channel;
use tracing::{error, info, trace, warn};

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
            .get(&TypeId::of::<rock_node_core::database_provider::DatabaseManagerProvider>())
            .and_then(|p| p.downcast_ref::<rock_node_core::database_provider::DatabaseManagerProvider>())
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
                    error!("Could not read latest persisted block from DB: {}. Retrying...", e);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
            };

            info!("Starting continuous backfill stream from block #{}", start_from);

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
                warn!("Failed to fill gap [{}, {}]: {}. Will retry on next cycle.", effective_start, end, e);
            } else {
                info!("Successfully filled gap [{}, {}].", effective_start, end);
            }
        }
        Ok(())
    }

    async fn establish_and_run_stream(&self, start: u64, end: u64) -> Result<()> {
        for peer_addr in &self.context.config.plugins.backfill.peers {
            info!("Connecting to peer {} for blocks [{}, {}]", peer_addr, start, end);
            match Channel::from_shared(peer_addr.clone())?.connect().await {
                Ok(channel) => {
                    let mut client = BlockStreamSubscribeServiceClient::new(channel);
                    let request = SubscribeStreamRequest {
                        start_block_number: start,
                        end_block_number: end,
                    };

                    let mut stream = client.subscribe_block_stream(request).await?.into_inner();
                    while let Some(msg_res) = stream.next().await {
                        match msg_res {
                            Ok(msg) => {
                                if let Some(SubResponse::BlockItems(item_set)) = msg.response {
                                    let block = Block { items: item_set.block_items };
                                    self.block_writer.write_block(&block)?;
                                }
                            }
                            Err(status) => return Err(anyhow!("gRPC stream error: {}", status)),
                        }
                    }
                    // If we get here, the stream finished successfully for this gap.
                    return Ok(());
                }
                Err(e) => {
                    warn!("Failed to connect to peer {}: {}. Trying next peer.", peer_addr, e);
                    continue;
                }
            }
        }
        Err(anyhow!("Failed to connect to any configured peers."))
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
