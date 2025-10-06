use async_trait::async_trait;
use prost::Message;
use rock_node_core::{
    app_context::AppContext,
    error::Result,
    events::{BlockItemsReceived, BlockVerificationFailed, BlockVerified},
    plugin::Plugin,
    Capability,
};
use rock_node_protobufs::com::hedera::hapi::block::stream::Block;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::{mpsc::Receiver, Notify};
use tracing::{error, info, trace, warn};

pub struct VerifierPlugin {
    context: Option<AppContext>,
    rx_block_items_received: Option<Receiver<BlockItemsReceived>>,
    running: Arc<AtomicBool>,
    shutdown_notify: Arc<Notify>,
}

impl VerifierPlugin {
    pub fn new(rx_block_items_received: Receiver<BlockItemsReceived>) -> Self {
        Self {
            context: None,
            rx_block_items_received: Some(rx_block_items_received),
            running: Arc::new(AtomicBool::new(false)),
            shutdown_notify: Arc::new(Notify::new()),
        }
    }
}

#[async_trait]
impl Plugin for VerifierPlugin {
    fn name(&self) -> &'static str {
        "verifier-plugin"
    }

    fn initialize(&mut self, context: AppContext) -> Result<()> {
        let registry = context.capability_registry.clone();

        // This task runs in the background to register our capability.
        tokio::spawn(async move {
            registry.register(Capability::ProvidesVerifiedBlocks).await;
        });

        self.context = Some(context);
        info!("VerifierPlugin initialized.");
        Ok(())
    }

    fn start(&mut self) -> Result<()> {
        info!("Starting VerifierPlugin event loop...");
        let context = self
            .context
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("VerifierPlugin not initialized"))?
            .clone();

        // Take ownership of the receiver end of the channel.
        let mut rx = self
            .rx_block_items_received
            .take()
            .ok_or_else(|| anyhow::anyhow!("VerifierPlugin receiver already taken"))?;

        let shutdown_notify = self.shutdown_notify.clone();
        let running_clone = self.running.clone();

        self.running.store(true, Ordering::SeqCst);
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown_notify.notified() => {
                        info!("VerifierPlugin received shutdown signal. Exiting loop.");
                        break;
                    }
                    Some(event) = rx.recv() => {
                        trace!(
                            "Verifier: Received block #{} with key [{}]. Verifying...",
                            event.block_number, event.cache_key
                        );

                        // Fetch data using the claim check.
                        let verification_result = if let Some(data) = context.block_data_cache.get(&event.cache_key) {
                            trace!("Verifier: Fetched data for block #{}: {} bytes", event.block_number, data.contents.len());

                            // Simulate verification work
                            tokio::time::sleep(Duration::from_millis(50)).await;

                            // Basic verification: try to decode the block
                            match Block::decode(data.contents.as_slice()) {
                                Ok(block_proto) => {
                                    // TODO: Add real verification logic here:
                                    // - Verify block hash
                                    // - Verify signatures
                                    // - Verify block proof
                                    // - Verify state transitions
                                    trace!("Verifier: Block #{} decoded successfully, {} items", event.block_number, block_proto.items.len());
                                    Ok(())
                                }
                                Err(e) => {
                                    error!("Verifier: Block #{} failed to decode: {}", event.block_number, e);
                                    Err(format!("Block decode failed: {}", e))
                                }
                            }
                        } else {
                            error!("Verifier: Block #{} data not found in cache", event.block_number);
                            Err("Block data not found in cache".to_string())
                        };

                        match verification_result {
                            Ok(()) => {
                                // Verification succeeded - send to persistence
                                let verified_event = BlockVerified {
                                    block_number: event.block_number,
                                    cache_key: event.cache_key,
                                };

                                if context.tx_block_verified.send(verified_event).await.is_err() {
                                    warn!("Verifier: Could not send verified block, persistence channel may be closed.");
                                }
                            }
                            Err(reason) => {
                                // Verification failed - notify publishers
                                error!("Verifier: Block #{} verification FAILED: {}", event.block_number, reason);

                                let failed_event = BlockVerificationFailed {
                                    block_number: event.block_number,
                                    cache_key: event.cache_key,
                                    reason: reason.clone(),
                                };

                                if context.tx_block_verification_failed.send(failed_event).is_err() {
                                    warn!("Verifier: Could not send verification failure event, no subscribers.");
                                }

                                // Also mark cache for removal since this block won't be persisted
                                context.block_data_cache.mark_for_removal(event.cache_key).await;
                            }
                        }
                    }
                    else => {
                        // Channel closed, exit loop
                        break;
                    }
                }
            }
            running_clone.store(false, Ordering::SeqCst);
            info!("VerifierPlugin event loop has terminated.");
        });
        Ok(())
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    async fn stop(&mut self) -> Result<()> {
        self.shutdown_notify.notify_waiters();
        self.running.store(false, Ordering::SeqCst);
        Ok(())
    }
}
