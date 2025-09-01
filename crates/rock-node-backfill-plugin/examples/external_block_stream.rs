use anyhow::Result;
use rock_node_backfill_plugin::stream_blocks_from_peers;
use tokio_stream::StreamExt;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    info!("🚀 Starting example of using the backfill crate as an external dependency.");
    info!("This example will attempt to stream blocks from a running rock-node peer.");
    info!("\nIMPORTANT: Please ensure you have a 'source' node running.");
    info!("You can start one with the command:");
    info!("cargo run -p rock-node -- --config-path source-config.toml\n");

    // 1. Define the peer to connect to.
    let peers = vec!["[http://127.0.0.1:6895](http://127.0.0.1:6895)".to_string()];

    // --- Case 1: Stream a finite, specific range ---
    let start_block = Some(10);
    let end_block = Some(20);

    info!(
        "Attempting to stream blocks [{:?}, {:?}] from peers: {:?}",
        start_block, end_block, peers
    );

    match stream_blocks_from_peers(&peers, start_block, end_block).await {
        Ok(mut block_stream) => {
            info!("✅ Successfully connected to peer and established finite stream.");
            while let Some(block_result) = block_stream.next().await {
                match block_result {
                    Ok(block) => {
                        let block_num = block.items.first().map_or(0, |item| {
                            if let Some(rock_node_protobufs::com::hedera::hapi::block::stream::block_item::Item::BlockHeader(h)) = &item.item {
                                h.number
                            } else {0}
                        });
                        info!("Received Block #{}", block_num);
                    }
                    Err(status) => {
                        tracing::error!("An error occurred during streaming: {}", status);
                        break;
                    }
                }
            }
            info!("Finite stream finished.");
        }
        Err(e) => {
            tracing::error!("Failed to establish a block stream: {}", e);
        }
    }

    // --- Case 2: Stream from the latest block onwards (continuous mode) ---
    let start_block_live = None;
    let end_block_live = None;

    info!(
        "\nAttempting to stream blocks from [{:?}] to [{:?}] from peers: {:?}",
        start_block_live, end_block_live, peers
    );

    match stream_blocks_from_peers(&peers, start_block_live, end_block_live).await {
        Ok(mut block_stream) => {
            info!("✅ Successfully connected to peer and established live stream.");
            let mut count = 0;
            while let Some(block_result) = block_stream.next().await {
                if let Ok(block) = block_result {
                    let block_num = block.items.first().map_or(0, |item| {
                        if let Some(rock_node_protobufs::com::hedera::hapi::block::stream::block_item::Item::BlockHeader(h)) = &item.item {
                            h.number
                        } else {0}
                    });
                    info!("Received live Block #{}", block_num);
                    count += 1;
                    if count > 5 {
                        // Stop after 5 live blocks for the example
                        break;
                    }
                }
            }
            info!("Live stream finished.");
        }
        Err(e) => {
            tracing::error!("Failed to establish a block stream: {}", e);
        }
    }

    info!("🎉 Example finished.");
    Ok(())
}
