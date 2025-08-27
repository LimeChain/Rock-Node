use anyhow::Result;
use async_trait::async_trait;
use rock_node_backfill_plugin::BackfillPlugin;
use rock_node_core::{
    app_context::AppContext,
    block_reader::{BlockReader, BlockReaderProvider},
    block_writer::{BlockWriter, BlockWriterProvider},
    config::{BackfillConfig, BackfillMode, Config, CoreConfig, PluginConfigs},
    database::DatabaseManager,
    database_provider::DatabaseManagerProvider,
    plugin::Plugin,
    test_utils::create_isolated_metrics,
};
use rock_node_protobufs::com::hedera::hapi::block::stream::Block;
use std::{
    any::{Any, TypeId},
    collections::HashMap,
    sync::{Arc, RwLock},
    time::Duration,
};
use tempfile::TempDir;
use tokio::sync::{broadcast, mpsc};
use tracing::info;

// --- User's Mock Implementations ---
// A user of the backfill plugin would provide their own implementations
// for reading from and writing to their specific database.

#[derive(Debug, Default)]
struct MyAppsBlockReader;
#[async_trait]
impl BlockReader for MyAppsBlockReader {
    fn get_latest_persisted_block_number(&self) -> Result<Option<u64>> {
        // Because this is only an example, we pretend it only has blocks up to 100.
        Ok(Some(100))
    }
    fn read_block(&self, n: u64) -> Result<Option<Vec<u8>>> {
        info!("[My App's Reader] The plugin asked for block #{}", n);
        Ok(None) // Return None to show it's missing.
    }
    fn get_earliest_persisted_block_number(&self) -> Result<Option<u64>> {
        Ok(Some(0))
    }
    fn get_highest_contiguous_block_number(&self) -> Result<u64> {
        Ok(100)
    }
}

#[derive(Debug, Default)]
struct MyAppsBlockWriter;
#[async_trait]
impl BlockWriter for MyAppsBlockWriter {
    async fn write_block(&self, block: &Block) -> Result<()> {
        // Because this is only an example, we don't write to a database.
        info!(
            "[My App's Writer] The plugin gave me a new block to write: {:?}",
            block
        );
        Ok(())
    }
    async fn write_block_batch(&self, blocks: &[Block]) -> Result<()> {
        info!(
            "[My App's Writer] The plugin gave me a batch of {} new blocks to write.",
            blocks.len()
        );
        Ok(())
    }
}

/// --- Main application entry point ---
#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    info!("🚀 Starting a minimal example of using the Backfill Plugin.");

    // A user needs a temporary directory for the plugin's internal state (like gaps).
    let temp_dir = TempDir::new()?;

    // 1. CONFIGURE THE PLUGIN
    // A user creates a configuration specifying their peer and backfill mode.
    let config = Config {
        core: CoreConfig {
            database_path: temp_dir.path().to_str().unwrap().to_string(),
            ..Default::default()
        },
        plugins: PluginConfigs {
            backfill: BackfillConfig {
                enabled: true,
                mode: BackfillMode::Continuous,
                peers: vec!["http://127.0.0.1:6895".to_string()], // A real or dummy peer address
                ..Default::default()
            },
            ..Default::default()
        },
    };

    // 2. PROVIDE REQUIRED DEPENDENCIES
    // The plugin needs access to a database and the user's block storage logic.
    let db_manager = Arc::new(DatabaseManager::new(&config.core.database_path)?);
    let service_providers = Arc::new(RwLock::new(
        HashMap::<TypeId, Arc<dyn Any + Send + Sync>>::new(),
    ));
    {
        let mut providers = service_providers.write().unwrap();
        providers.insert(
            TypeId::of::<DatabaseManagerProvider>(),
            Arc::new(DatabaseManagerProvider::new(db_manager)),
        );
        providers.insert(
            TypeId::of::<BlockReaderProvider>(),
            Arc::new(BlockReaderProvider::new(Arc::new(
                MyAppsBlockReader::default(),
            ))),
        );
        providers.insert(
            TypeId::of::<BlockWriterProvider>(),
            Arc::new(BlockWriterProvider::new(Arc::new(
                MyAppsBlockWriter::default(),
            ))),
        );
    }

    // 3. CREATE THE MINIMAL APP CONTEXT
    // This context object provides the plugin with everything it needs to operate.
    let (tx_persisted, _) = broadcast::channel(1);
    let (tx_items, _) = mpsc::channel(1);
    let (tx_verified, _) = mpsc::channel(1);
    let app_context = AppContext {
        config: Arc::new(config),
        metrics: Arc::new(create_isolated_metrics()),
        capability_registry: Arc::new(Default::default()),
        service_providers,
        block_data_cache: Arc::new(Default::default()),
        tx_block_items_received: tx_items,
        tx_block_verified: tx_verified,
        tx_block_persisted: tx_persisted,
    };

    // 4. INITIALIZE AND START THE PLUGIN
    let mut backfill_plugin = BackfillPlugin::new();
    info!("Initializing the plugin...");
    backfill_plugin.initialize(app_context)?;

    info!("Starting the plugin...");
    backfill_plugin.start()?;

    // 5. LET IT RUN
    info!("✅ Plugin is now running in Continuous mode.");
    info!("It will try to fetch blocks starting from #101 from its peer.");
    info!(
        "If the peer is running, you'll see the plugin fetching blocks, else a handful of errors."
    );
    info!("This demonstrates the plugin is active and working as intended.");
    info!("Running for 15 seconds...");

    tokio::time::sleep(Duration::from_secs(15)).await;

    // 6. SHUTDOWN
    info!("Stopping the plugin...");
    backfill_plugin.stop().await?;

    info!("🎉 Example finished.");
    Ok(())
}
