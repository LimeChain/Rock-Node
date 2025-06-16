// rock-node-workspace/app/rock-node/src/main.rs

use rock_node_publish_plugin::PublishPlugin;
use rock_node_verifier_plugin::VerifierPlugin;
use rock_node_persistence_plugin::PersistencePlugin;
use serde_json;
use anyhow::Result;
use rock_node_core::app_context::AppContext;
use rock_node_core::capability::CapabilityRegistry;
use rock_node_core::config::Config;
use rock_node_core::{events, BlockDataCache, Plugin};
use rock_node_observability_plugin::ObservabilityPlugin;
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio::sync::{broadcast, mpsc};
use tracing::info;

const BANNER: &str = r#"
██████╗  ██████╗  ██████╗██╗  ██╗    ███╗   ██╗ ██████╗ ██████╗ ███████╗
██╔══██╗██╔═══██╗██╔════╝██║ ██╔╝    ████╗  ██║██╔═══██╗██╔══██╗██╔════╝
██████╔╝██║   ██║██║     █████╔╝     ██╔██╗ ██║██║   ██║██║  ██║█████╗  
██╔══██╗██║   ██║██║     ██╔═██╗     ██║╚██╗██║██║   ██║██║  ██║██╔══╝  
██║  ██║╚██████╔╝╚██████╗██║  ██╗    ██║ ╚████║╚██████╔╝██████╔╝███████╗
╚═╝  ╚═╝ ╚═════╝  ╚═════╝╚═╝  ╚═╝    ╚═╝  ╚═══╝ ╚═════╝ ╚═════╝ ╚══════╝
"#;

/// Custom formatter for Config to make it more human-readable
fn format_config(config: &Config) -> String {
    // Convert the config to a serde_json Value for easy traversal
    let json_value = serde_json::to_value(config)
        .expect("Failed to serialize config to JSON");
    
    let mut output = String::new();
    output.push_str("==== Configuration ====\n");
    
    // Helper function to recursively format the config
    fn format_value(path: &str, value: &serde_json::Value, output: &mut String) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, val) in map {
                    let new_path = if path.is_empty() {
                        key.to_string()
                    } else {
                        format!("{}.{}", path, key)
                    };
                    format_value(&new_path, val, output);
                }
            }
            serde_json::Value::Array(arr) => {
                for (i, val) in arr.iter().enumerate() {
                    let new_path = format!("{}[{}]", path, i);
                    format_value(&new_path, val, output);
                }
            }
            _ => {
                // For primitive values, format as key = value
                let value_str = match value {
                    serde_json::Value::String(s) => format!("\"{}\"", s),
                    _ => value.to_string(),
                };
                output.push_str(&format!("{} = {}\n", path, value_str));
            }
        }
    }
    
    format_value("", &json_value, &mut output);
    
    output
}

#[tokio::main]
async fn main() -> Result<()> {
    // --- Step 1: Initialize Logging ---
    // We set up a subscriber that listens for `tracing` events.
    // We use `EnvFilter` to allow log levels to be set via the `RUST_LOG`
    // environment variable, defaulting to the level in our config.
    let config_path = "config/development.toml";
    let config_str = std::fs::read_to_string(config_path)
        .map_err(|e| anyhow::anyhow!("Failed to read config file at '{}': {}", config_path, e))?;
    
    let temp_config: toml::Value = toml::from_str(&config_str)?;
    let default_log_level = temp_config["core"]["log_level"].as_str().unwrap_or("INFO");

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env().add_directive(
            default_log_level.parse()?,
        ))
        .init();
    
    // --- Step 2: Print Banner and Load Final Config ---
    println!("{}", BANNER);

    // Now, deserialize the full configuration string into our strongly-typed struct.
    let config: Config = toml::from_str(&config_str)?;

    // Use our custom formatter for a more human-readable configuration display
    info!("{}", format_config(&config));

    // --- Step 3: Build the AppContext ---
    // --- Create Channels and Core Facilities ---
    let (tx_items, rx_items) = mpsc::channel::<events::BlockItemsReceived>(100);
    let (tx_verified, rx_verified) = mpsc::channel::<events::BlockVerified>(100);
    let (tx_persisted, _) = broadcast::channel::<events::BlockPersisted>(100);

    info!("Building application context...");
    let app_context = AppContext {
        config: Arc::new(config),
        capability_registry: Arc::new(CapabilityRegistry::new()),
        service_providers: Arc::new(RwLock::new(HashMap::<TypeId, Arc<dyn Any + Send + Sync>>::new())),
        block_data_cache: Arc::new(BlockDataCache::new()),
        tx_block_items_received: tx_items,
        tx_block_verified: tx_verified,
        tx_block_persisted: tx_persisted,
    };

    let mut plugins: Vec<Box<dyn Plugin>> = vec![];

    // Read the verifier config *before* moving channels
    let verification_service_enabled = app_context.config.plugins.verification_service.enabled;

    if verification_service_enabled {
        info!("VerifierPlugin is ENABLED. Wiring Verifier -> Persistence.");
        // PersistencePlugin gets None for rx_items, since it will use rx_verified
        plugins.push(Box::new(PersistencePlugin::new(None, rx_verified)));
        // VerifierPlugin takes ownership of rx_items
        plugins.push(Box::new(VerifierPlugin::new(rx_items)));

    } else {
        info!("VerifierPlugin is DISABLED. Wiring directly to Persistence.");
        // PersistencePlugin takes ownership of rx_items
        plugins.push(Box::new(PersistencePlugin::new(Some(rx_items), rx_verified)));
    }
    
    plugins.push(Box::new(ObservabilityPlugin::new()));
    plugins.push(Box::new(PublishPlugin::new()));

    // --- Step 4: Instantiate and Initialize Plugins ---
    info!("Initializing plugins...");
    for plugin in &mut plugins {
        plugin.initialize(app_context.clone())?;
    }

    // --- Step 5: Start Plugins ---
    info!("Starting plugins...");
    for plugin in &mut plugins {
        plugin.start()?;
    }

    info!("Rock Node running successfully.");
    
    // Keep the main thread alive, waiting for a shutdown signal (e.g., Ctrl+C)
    tokio::signal::ctrl_c().await?;
    
    Ok(())
}
