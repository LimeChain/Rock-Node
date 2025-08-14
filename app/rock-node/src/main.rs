use anyhow::Result;
use clap::Parser;
use config as config_rs;
use dotenvy::dotenv;
use rock_node_block_access_plugin::BlockAccessPlugin;
use rock_node_core::{
    app_context::AppContext, capability::CapabilityRegistry, config::Config as RockConfig,
    database::DatabaseManager, database_provider::DatabaseManagerProvider, events, BlockDataCache,
    MetricsRegistry, Plugin,
};
use rock_node_observability_plugin::ObservabilityPlugin;
use rock_node_persistence_plugin::PersistencePlugin;
use rock_node_publish_plugin::PublishPlugin;
use rock_node_query_plugin::QueryPlugin;
use rock_node_server_status_plugin::StatusPlugin;
use rock_node_state_management_plugin::StateManagementPlugin;
use rock_node_subscriber_plugin::SubscriberPlugin;
use rock_node_verifier_plugin::VerifierPlugin;
use rock_node_backfill_plugin::BackfillPlugin;
use std::{
    any::{Any, TypeId},
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, RwLock},
    time::Duration,
};
use tokio::sync::{broadcast, mpsc};
use tracing::{error, info};

fn print_section(name: &str, section: &toml::Value, depth: usize) {
    let dash_char = if name == "plugins" && depth == 0 {
        '='
    } else {
        '-'
    };
    let dash_count = 8 + depth;
    let dashes = dash_char.to_string().repeat(dash_count);
    info!("{} {} {}", dashes, name.to_uppercase(), dashes);

    if let toml::Value::Table(t) = section {
        for (key, value) in t.iter() {
            if let toml::Value::Table(_) = value {
                print_section(key, value, depth + 1);
            } else {
                info!("{}: {}", key, value);
            }
        }
    }

    info!("{}", "-".repeat(16));
}

fn print_config(config_value: &toml::Value) {
    info!("===== Configuration =====");

    if let toml::Value::Table(top) = config_value {
        for (name, val) in top.iter() {
            print_section(name, val, 0);
        }
    }
}

const BANNER: &str = r#"
██████╗  ██████╗  ██████╗██╗  ██╗    ███╗   ██╗ ██████╗ ██████╗ ███████╗
██╔══██╗██╔═══██╗██╔════╝██║ ██╔╝    ████╗  ██║██════██╗██╔══██╗██╔════╝
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

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {info!("Received Ctrl+C, initiating shutdown...");},
        _ = terminate => {info!("Received SIGTERM, initiating shutdown...");},
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // --- Step 1: Parse Command-Line Arguments ---
    let args = Args::parse();

    // --- Step 2: Load Config and Initialize Logging (ONCE) ---
    // NOTE: We no longer need to read the config TOML manually; the `config` crate will handle it.
    //       We'll still validate that the file exists early to provide a clear error message.
    if !args.config_path.exists() {
        anyhow::bail!("Config file not found at '{}'", args.config_path.display());
    }

    // Load environment variables from .env (if present) and the process environment.
    dotenv().ok();

    // Build layered configuration: file values first, then environment variable overrides.
    let settings = config_rs::Config::builder()
        .add_source(config_rs::File::from(args.config_path.clone()))
        .add_source(config_rs::Environment::with_prefix("ROCK_NODE").separator("__"))
        .build()?;

    // Deserialize twice: once into a TOML Value for pretty-printing, and once into our typed struct.
    let merged_config_value: toml::Value = settings.clone().try_deserialize()?;
    let config: RockConfig = settings.try_deserialize()?;
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
    info!("Configuration loaded successfully (after env overrides).");
    print_config(&merged_config_value);

    // --- Step 4: Initialize Core Shared Services ---
    info!("Initializing shared services...");
    let db_manager = Arc::new(DatabaseManager::new(&config.core.database_path)?);
    info!("DatabaseManager initialized.");

    let service_providers = Arc::new(RwLock::new(
        HashMap::<TypeId, Arc<dyn Any + Send + Sync>>::new(),
    ));

    {
        let mut providers = service_providers.write().map_err(|e| {
            anyhow::anyhow!("Failed to acquire write lock on service providers: {}", e)
        })?;
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
    plugins.push(Box::new(SubscriberPlugin::new()));
    plugins.push(Box::new(BlockAccessPlugin::new()));
    plugins.push(Box::new(StatusPlugin::new()));
    plugins.push(Box::new(ObservabilityPlugin::new()));
    plugins.push(Box::new(StateManagementPlugin::new()));
    plugins.push(Box::new(QueryPlugin::new()));
    plugins.push(Box::new(BackfillPlugin::new()));
    
    // --- Step 7: Initialize and Start Plugins ---
    info!("{}", "-".repeat(16));
    info!("Initializing plugins...");
    for plugin in &mut plugins {
        plugin.initialize(app_context.clone())?;
    }
    info!("{}", "-".repeat(16));
    info!("Starting plugins...");
    for plugin in &mut plugins {
        plugin.start()?;
    }
    info!("{}", "-".repeat(16));
    info!("Rock Node running successfully!");

    // --- Wait for Shutdown Signal ---
    shutdown_signal().await;

    // --- Step 8: Graceful Shutdown ---
    info!("Shutdown signal received. Stopping plugins...");
    // Iterate in reverse order of startup to handle dependencies correctly.
    for plugin in plugins.iter_mut().rev() {
        if plugin.is_running() {
            info!("Stopping plugin '{}'...", plugin.name());
            if let Err(e) = plugin.stop().await {
                error!("Error stopping plugin '{}': {}", plugin.name(), e);
            }
        }
    }

    // A small delay to allow background tasks to finish logging, etc.
    tokio::time::sleep(Duration::from_millis(250)).await;
    info!("All plugins stopped. Exiting.");

    Ok(())
}
