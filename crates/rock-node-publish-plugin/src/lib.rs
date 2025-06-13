// File: rock-node-workspace/crates/rock-node-publish-plugin/src/lib.rs
// (Corrected Version)

use rock_node_core::{
    app_context::AppContext,
    error::Result,
    events::{BlockData, BlockItemsReceived},
    plugin::Plugin,
};
use std::time::Duration;
use tokio::time::sleep;
use tracing::info;

#[derive(Debug, Default)]
pub struct PublishPlugin {
    context: Option<AppContext>,
}

impl PublishPlugin {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Plugin for PublishPlugin {
    fn name(&self) -> &'static str {
        "publish-plugin"
    }

    fn initialize(&mut self, context: AppContext) -> Result<()> {
        self.context = Some(context);
        info!("PublishPlugin initialized.");
        Ok(())
    }

    fn start(&mut self) -> Result<()> {
        info!("Starting PublishPlugin...");
        let context = self.context.as_ref().unwrap().clone();
        
        // This task simulates a new block being produced every 5 seconds.
        tokio::spawn(async move {
            let mut block_number = 0;
            loop {
                sleep(Duration::from_secs(5)).await;
                
                let block_data = BlockData {
                    block_number,
                    contents: format!("This is the data for block #{}", block_number),
                };
                info!("Publishing new block: #{}", block_number);

                // "Claim Check" Pattern: Store the data and get a key.
                let cache_key = context.block_data_cache.insert(block_data);

                // Publish the event with the claim check key.
                let event = BlockItemsReceived {
                    block_number,
                    cache_key,
                };
                
                if context.tx_block_items_received.send(event).await.is_err() {
                    // This can happen if there are no subscribers, which is okay.
                    // In a real app, we might log a warning here.
                }
                
                block_number += 1;
            }
        });
        Ok(())
    }
}
