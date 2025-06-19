use anyhow::Result;
use clap::Parser;
use rock_node_block_access_plugin::BlockAccessPlugin;
use rock_node_core::{
    app_context::AppContext, capability::CapabilityRegistry, config::Config,
    database::DatabaseManager, database_provider::DatabaseManagerProvider, events, BlockDataCache,
    MetricsRegistry, Plugin,
};
use rock_node_observability_plugin::ObservabilityPlugin;
use rock_node_persistence_plugin::PersistencePlugin;
use rock_node_publish_plugin::PublishPlugin;
use rock_node_server_status_plugin::StatusPlugin;
use rock_node_verifier_plugin::VerifierPlugin;
use std::{
    any::{Any, TypeId},
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, RwLock},
};
use tokio::sync::{broadcast, mpsc};
use tracing::info;

const BANNER: &str = r#"
██████╗  ██████╗  ██████╗██╗  ██╗    ███╗   ██╗ ██████╗ ██████╗ ███████╗
██╔══██╗██╔═══██╗██╔════╝██║ ██╔╝    ████╗  ██║██═══██╗██╔══██╗██╔════╝
██████╔╝██║   ██║██║     █████╔╝     ██╔██╗ ██║██║   ██║██║  ██║█████╗  
██╔══██╗██║   ██║██║     ██╔═██╗     ██║╚██╗██║██║   ██║██║  ██║██╔══╝  
██║  ██║╚██████╔╝╚██████╗██║  ██╗    ██║ ╚████║╚██████╔╝██████╔╝███████╗
╚═╝  ╚═╝ ╚═════╝  ╚═════╝╚═╝  ╚═╝    ╚═╝  ╚═══╝ ╚═════╝ ╚═════╝ ╚══════╝
"#;

/// Defines command-line arguments for the application.
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Path to the TOML configuration file.
    #[arg(long, default_value = "config/config.toml")]
    config_path: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    // --- Step 1: Parse Command-Line Arguments ---
    let args = Args::parse();

    // --- Step 2: Load Config and Initialize Logging (ONCE) ---
    let config_str = std::fs::read_to_string(&args.config_path).map_err(|e| {
        anyhow::anyhow!(
            "Failed to read config file at '{}': {}",
            args.config_path.display(),
            e
        )
    })?;

    let config: Config = toml::from_str(&config_str)?;
    let config = Arc::new(config); // Wrap config in an Arc early
    let default_log_level = &config.core.log_level;

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(default_log_level.parse()?),
        )
        .init();

    info!(
        "Logger initialized with level '{}'. Loading from: {:?}",
        default_log_level, &args.config_path
    );

    // --- Step 3: Print Banner and Show Config ---
    println!("{}", BANNER);
    info!("Configuration loaded successfully.\n{:#?}", config);

    // --- Step 4: Initialize Core Shared Services ---
    info!("Initializing shared services...");
    let db_manager = Arc::new(DatabaseManager::new(&config.core.database_path)?);
    info!("DatabaseManager initialized.");

    let service_providers = Arc::new(RwLock::new(
        HashMap::<TypeId, Arc<dyn Any + Send + Sync>>::new(),
    ));

    {
        let mut providers = service_providers.write().unwrap();
        // Create the provider wrapper and store an Arc to IT.
        let db_manager_provider = DatabaseManagerProvider::new(db_manager);
        providers.insert(
            TypeId::of::<DatabaseManagerProvider>(), // The key is the TypeId of the PROVIDER
            Arc::new(db_manager_provider),
        );
        info!("DatabaseManagerProvider registered as a core service provider.");
    }

    // --- Step 5: Build the AppContext ---
    let (tx_items, rx_items) = mpsc::channel::<events::BlockItemsReceived>(100);
    let (tx_verified, rx_verified) = mpsc::channel::<events::BlockVerified>(100);
    let (tx_persisted, _) = broadcast::channel::<events::BlockPersisted>(100);

    info!("Building application context...");
    let app_context = AppContext {
        config: config.clone(),
        metrics: Arc::new(MetricsRegistry::new()?),
        capability_registry: Arc::new(CapabilityRegistry::new()),
        service_providers,
        block_data_cache: Arc::new(BlockDataCache::new()),
        tx_block_items_received: tx_items,
        tx_block_verified: tx_verified,
        tx_block_persisted: tx_persisted,
    };

    // --- Step 6: Assemble Plugins ---
    let mut plugins: Vec<Box<dyn Plugin>> = vec![];
    let verification_service_enabled = app_context.config.plugins.verification_service.enabled;

    if verification_service_enabled {
        info!("VerifierPlugin is ENABLED. Wiring Verifier -> Persistence.");
        plugins.push(Box::new(PersistencePlugin::new(None, rx_verified)));
        plugins.push(Box::new(VerifierPlugin::new(rx_items)));
    } else {
        info!("VerifierPlugin is DISABLED. Wiring directly to Persistence.");
        plugins.push(Box::new(PersistencePlugin::new(
            Some(rx_items),
            rx_verified,
        )));
    }

    plugins.push(Box::new(PublishPlugin::new()));
    plugins.push(Box::new(BlockAccessPlugin::new()));
    plugins.push(Box::new(StatusPlugin::new()));
    plugins.push(Box::new(ObservabilityPlugin::new()));

    // --- Step 7: Initialize and Start Plugins ---
    info!("Initializing plugins...");
    for plugin in &mut plugins {
        plugin.initialize(app_context.clone())?;
    }

    info!("Starting plugins...");
    for plugin in &mut plugins {
        plugin.start()?;
    }

    info!("Rock Node running successfully. Press Ctrl+C to shut down.");
    tokio::signal::ctrl_c().await?;
    info!("Shutdown signal received. Exiting.");

    Ok(())
}
