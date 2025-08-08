use rock_node_core::{
    app_context::AppContext,
    events::{BlockItemsReceived, FilteredBlock, FilteredBlockReady},
};
use std::sync::Arc;
use tokio::sync::{broadcast, Notify};
use tracing::{debug, info};

/// Manages the block filtering process.
pub struct FilterManager {
    context: AppContext,
    rx_block_items: broadcast::Receiver<BlockItemsReceived>,
    shutdown_notify: Arc<Notify>,
}

impl FilterManager {
    pub fn new(
        context: AppContext,
        rx_block_items: broadcast::Receiver<BlockItemsReceived>,
        shutdown_notify: Arc<Notify>,
    ) -> Self {
        Self {
            context,
            rx_block_items,
            shutdown_notify,
        }
    }

    /// The main entry point for the filter task.
    pub async fn run(mut self) {
        info!("Filter Manager started.");
        loop {
            tokio::select! {
                _ = self.shutdown_notify.notified() => {
                    info!("Filter Manager received shutdown signal. Exiting.");
                    break;
                }
                Ok(event) = self.rx_block_items.recv() => {
                    self.process_block(event).await;
                }
                else => {
                    // The main channel has closed.
                    info!("Primary block channel closed. Filter Manager is terminating.");
                    break;
                }
            }
        }
        info!("Filter Manager has terminated.");
    }

    /// Processes a single block event from the primary stream.
    async fn process_block(&self, event: BlockItemsReceived) {
        debug!("Filter processing block #{}", event.block_number);

        // In a real implementation, you would fetch the block data from the cache:
        // let block_data = self.context.block_data_cache.get(&event.cache_key);
        // ... then apply filtering logic ...

        // For now, we'll just create a placeholder filtered block.
        let filtered_block = FilteredBlock {
            block_number: event.block_number,
        };

        let filtered_event = FilteredBlockReady { filtered_block };

        if self
            .context
            .tx_filtered_block_ready
            .send(filtered_event)
            .is_err()
        {
            debug!("No listeners for the filtered block channel.");
        }
    }
}
