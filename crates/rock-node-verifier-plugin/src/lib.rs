// File: rock-node-workspace/crates/rock-node-verifier-plugin/src/lib.rs

use std::time::Duration;

use rock_node_core::{
    app_context::AppContext,
    error::Result,
    events::{BlockItemsReceived, BlockVerified},
    plugin::Plugin,
    Capability,
};
use tokio::sync::mpsc::Receiver;
use tracing::info;

pub struct VerifierPlugin {
    context: Option<AppContext>,
    rx_block_items_received: Receiver<BlockItemsReceived>,
}

impl VerifierPlugin {
    // It receives the "read" end of the channel at creation.
    pub fn new(rx_block_items_received: Receiver<BlockItemsReceived>) -> Self {
        Self {
            context: None,
            rx_block_items_received,
        }
    }
}

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
        info!("Starting VerifierPlugin...");
        let context = self.context.as_ref().unwrap().clone();
        
        // Take ownership of the receiver end of the channel.
        let mut rx = std::mem::replace(&mut self.rx_block_items_received, tokio::sync::mpsc::channel(1).1);

        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                info!(
                    "Verifier: Received block #{} with key [{}]. Verifying...",
                    event.block_number, event.cache_key
                );

                // Fetch data using the claim check.
                if let Some(data) = context.block_data_cache.get(&event.cache_key) {
                    info!("Verifier: Fetched data: '{}'", data.contents);
                }
                
                // Pretend to do work
                tokio::time::sleep(Duration::from_millis(50)).await;

                let verified_event = BlockVerified {
                    block_number: event.block_number,
                    cache_key: event.cache_key,
                };
                
                if context.tx_block_verified.send(verified_event).await.is_err() {
                     // No subscriber for verified blocks.
                }
            }
        });
        Ok(())
    }
}
