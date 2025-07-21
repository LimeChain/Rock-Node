use async_trait::async_trait;
use rock_node_core::{
    app_context::AppContext,
    error::Result,
    events::{BlockItemsReceived, BlockVerified},
    plugin::Plugin,
    Capability,
};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::{mpsc::Receiver, Notify};
use tracing::{info, trace};

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
                        if let Some(data) = context.block_data_cache.get(&event.cache_key) {
                            trace!("Verifier: Fetched data for block #{}: '{:?}'", event.block_number, data.contents.len());
                        }

                        // Pretend to do work
                        tokio::time::sleep(Duration::from_millis(50)).await;

                        let verified_event = BlockVerified {
                            block_number: event.block_number,
                            cache_key: event.cache_key,
                        };

                        if context.tx_block_verified.send(verified_event).await.is_err() {
                            // This can happen if the persistence plugin shuts down first.
                            info!("Verifier: Could not send verified block, persistence channel may be closed.");
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
