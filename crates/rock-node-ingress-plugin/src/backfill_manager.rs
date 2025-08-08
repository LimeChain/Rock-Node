use crate::state::SharedState;
use anyhow::{anyhow, Result};
use prost::Message;
use rock_node_core::{app_context::AppContext, BlockReaderProvider};
use rock_node_protobufs::{
    com::hedera::hapi::block::stream::{block_item, Block, BlockItem},
    org::hiero::block::api::{
        block_stream_subscribe_service_client::BlockStreamSubscribeServiceClient,
        subscribe_stream_response::Response as ResponseType, SubscribeStreamRequest,
    },
};
use std::{any::TypeId, sync::Arc, time::Duration};
use tokio::sync::Notify;
use tracing::{debug, error, info, trace, warn};

/// Manages the process of finding and filling gaps in the block history.
pub struct BackfillManager {
    context: AppContext,
    shared_state: Arc<SharedState>,
    shutdown_notify: Arc<Notify>,
}

impl BackfillManager {
    pub fn new(
        context: AppContext,
        shared_state: Arc<SharedState>,
        shutdown_notify: Arc<Notify>,
    ) -> Self {
        Self {
            context,
            shared_state,
            shutdown_notify,
        }
    }

    /// The main entry point for the backfill task.
    pub async fn run(self) {
        info!("Backfill Manager started.");
        let check_interval = Duration::from_secs(60); // Check for new gaps every minute

        loop {
            tokio::select! {
                _ = self.shutdown_notify.notified() => {
                    info!("Backfill Manager received shutdown signal. Exiting.");
                    break;
                }
                _ = tokio::time::sleep(check_interval) => {
                    if let Err(e) = self.run_gap_detection_and_fill().await {
                        warn!("Error during backfill cycle: {}", e);
                    }
                }
            }
        }
        info!("Backfill Manager has terminated.");
    }

    /// Scans for gaps and attempts to fill them.
    async fn run_gap_detection_and_fill(&self) -> Result<()> {
        let gaps = self.detect_gaps()?;
        if gaps.is_empty() {
            trace!("No gaps detected in block storage.");
            return Ok(());
        }

        info!(
            "Detected {} gaps in block storage. Starting backfill.",
            gaps.len()
        );

        for (start, end) in gaps {
            info!("Attempting to fill gap from block #{} to #{}.", start, end);
            if let Err(e) = self.fill_gap(start, end).await {
                error!("Failed to fill gap from #{} to #{}: {}", start, end, e);
            }
        }
        Ok(())
    }

    /// Detects all ranges of missing blocks.
    fn detect_gaps(&self) -> Result<Vec<(u64, u64)>> {
        let providers = self.context.service_providers.read().unwrap();
        let block_reader = providers
            .get(&TypeId::of::<BlockReaderProvider>())
            .and_then(|p| p.downcast_ref::<BlockReaderProvider>())
            .ok_or_else(|| anyhow!("BlockReaderProvider not found for backfill"))?
            .get_reader();

        if block_reader
            .get_earliest_persisted_block_number()?
            .is_none()
        {
            return Ok(vec![]); // No blocks, so no gaps
        }

        let latest = block_reader
            .get_latest_persisted_block_number()?
            .unwrap_or(0);
        let highest_contiguous = block_reader.get_highest_contiguous_block_number()?;

        if latest <= highest_contiguous {
            return Ok(vec![]); // No gaps
        }

        let mut gaps = Vec::new();
        let current = highest_contiguous + 1;

        if latest > current {
            gaps.push((current, latest - 1));
        }

        Ok(gaps)
    }

    /// Connects to peers and streams blocks to fill a specific gap.
    async fn fill_gap(&self, start_block: u64, end_block: u64) -> Result<()> {
        let peers = &self.context.config.plugins.ingress_service.backfill.peers;
        if peers.is_empty() {
            return Err(anyhow!("No backfill peers configured."));
        }

        for peer_address in peers {
            info!("Attempting to backfill from peer: {}", peer_address);
            match self
                .stream_from_peer(peer_address, start_block, end_block)
                .await
            {
                Ok(_) => {
                    info!(
                        "Successfully filled gap from #{} to #{} using peer {}",
                        start_block, end_block, peer_address
                    );
                    return Ok(());
                }
                Err(e) => {
                    warn!(
                        "Failed to backfill from peer {}: {}. Trying next peer.",
                        peer_address, e
                    );
                }
            }
        }

        Err(anyhow!(
            "Failed to fill gap from #{} to #{} after trying all available peers.",
            start_block,
            end_block
        ))
    }

    async fn stream_from_peer(
        &self,
        address: &str,
        start_block: u64,
        end_block: u64,
    ) -> Result<()> {
        let mut client = BlockStreamSubscribeServiceClient::connect(address.to_string()).await?;
        let request = SubscribeStreamRequest {
            start_block_number: start_block,
            end_block_number: end_block,
        };

        let mut stream = client.subscribe_block_stream(request).await?.into_inner();

        while let Some(response_result) = stream.message().await? {
            if let Some(ResponseType::BlockItems(item_set)) = response_result.response {
                let block = self.reconstruct_block(item_set.block_items)?;
                let block_number = block
                    .items
                    .first()
                    .and_then(|item| match &item.item {
                        Some(block_item::Item::BlockHeader(h)) => Some(h.number),
                        _ => None,
                    })
                    .ok_or_else(|| anyhow!("Received block without a header from peer"))?;

                debug!("Received backfilled block #{}", block_number);

                let block_data = rock_node_core::events::BlockData {
                    block_number,
                    contents: block.encode_to_vec(),
                };
                let cache_key = self.context.block_data_cache.insert(block_data);
                let event = rock_node_core::events::BlockItemsReceived {
                    block_number,
                    cache_key,
                };
                self.context.tx_block_items_received.send(event).await?;
            }
        }

        Ok(())
    }

    /// Reconstructs a `Block` from a `Vec<BlockItem>`.
    fn reconstruct_block(&self, items: Vec<BlockItem>) -> Result<Block> {
        if items.is_empty() {
            return Err(anyhow!("Received empty BlockItemSet from peer"));
        }
        Ok(Block { items })
    }
}
