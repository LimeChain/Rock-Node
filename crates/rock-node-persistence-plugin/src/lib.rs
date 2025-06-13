// File: rock-node-workspace/crates/rock-node-plugin-persistence/src/lib.rs

use std::time::Duration;

use rock_node_core::{
    app_context::AppContext,
    error::Result,
    events::BlockVerified,
    plugin::Plugin,
};
use tokio::sync::mpsc::Receiver;
use tracing::info;

pub struct PersistencePlugin {
    context: Option<AppContext>,
    rx_block_verified: Receiver<BlockVerified>,
}

impl PersistencePlugin {
    pub fn new(rx_block_verified: Receiver<BlockVerified>) -> Self {
        Self {
            context: None,
            rx_block_verified,
        }
    }
}

impl Plugin for PersistencePlugin {
    fn name(&self) -> &'static str {
        "persistence-plugin"
    }

    fn initialize(&mut self, context: AppContext) -> Result<()> {
        self.context = Some(context);
        info!("PersistencePlugin initialized.");
        Ok(())
    }

    fn start(&mut self) -> Result<()> {
        info!("Starting PersistencePlugin...");
        let context = self.context.as_ref().unwrap().clone();
        let mut rx = std::mem::replace(&mut self.rx_block_verified, tokio::sync::mpsc::channel(1).1);

        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                info!(
                    "Persistence: Received verified block #{} with key [{}]. Persisting...",
                    event.block_number, event.cache_key
                );

                if let Some(data) = context.block_data_cache.get(&event.cache_key) {
                     info!("Persistence: Fetched data: '{}'", data.contents);
                }

                // Pretend to do work
                tokio::time::sleep(Duration::from_millis(100)).await;
                
                // Final step: remove the data from the cache.
                context.block_data_cache.remove(&event.cache_key);
                info!("Persistence: Evicted key [{}] from cache.", event.cache_key);
                
                // We could publish a final `BlockPersisted` event here if needed.
            }
        });

        Ok(())
    }
}
