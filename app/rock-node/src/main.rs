use anyhow::Result;
use clap::Parser;
use config as config_rs;
use rock_node_backfill_plugin::BackfillPlugin;
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
use std::{
    any::{Any, TypeId},
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, RwLock},
    time::Duration,
};
use tokio::sync::{broadcast, mpsc, watch};
use tonic::{service::RoutesBuilder, transport::Server};
use tracing::{error, info};

/// Reports the startup status of all plugins with detailed information.
pub fn report_plugin_startup_status(plugins: &[Box<dyn Plugin>]) {
    let mut failed_plugins = Vec::new();
    let mut running_plugins = Vec::new();

    for plugin in plugins {
        if plugin.is_running() {
            running_plugins.push(plugin.name().to_string());
        } else {
            failed_plugins.push(plugin.name().to_string());
        }
    }

    let (message, _) =
        analyze_plugin_startup_results(plugins.len(), &running_plugins, &failed_plugins);
    info!("{}", message);

    if failed_plugins.is_empty() {
        info!("Rock Node running successfully!");
    } else {
        info!("Rock Node running successfully! However, some plugins may not be available!");
    }
}

/// Validates that a configuration file exists and is readable.
pub fn validate_config_file_exists(config_path: &PathBuf) -> Result<(), anyhow::Error> {
    if !config_path.exists() {
        anyhow::bail!("Config file not found at '{}'", config_path.display());
    }
    Ok(())
}

/// Creates a formatted plugin list for logging purposes.
pub fn format_plugin_list(plugin_names: &[String], status_symbol: &str) -> String {
    if plugin_names.is_empty() {
        return "None".to_string();
    }
    plugin_names
        .iter()
        .map(|name| {
            format!(
                "  {} {} - {}",
                status_symbol,
                name,
                if status_symbol == "✅" {
                    "OK"
                } else {
                    "FAILED"
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Analyzes plugin startup results and generates a summary report.
pub fn analyze_plugin_startup_results(
    total_plugins: usize,
    running_plugins: &[String],
    failed_plugins: &[String],
) -> (String, bool) {
    let all_successful = failed_plugins.is_empty();
    let message = if all_successful {
        format!("✅ All {} plugins started successfully!", total_plugins)
    } else {
        let failed_list = format_plugin_list(failed_plugins, "❌");
        let running_list = format_plugin_list(running_plugins, "✅");
        format!(
            "⚠️  Some plugins failed to start:\n{}\n✅ {} plugins running:\n{}",
            failed_list,
            running_plugins.len(),
            running_list
        )
    };
    (message, all_successful)
}

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

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
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
    // --- Step 1 & 2: Parse Args, Load Config, Initialize Logging ---
    let args = Args::parse();
    validate_config_file_exists(&args.config_path)?;
    dotenvy::dotenv().ok();
    let settings = config_rs::Config::builder()
        .add_source(config_rs::File::from(args.config_path.clone()))
        .add_source(config_rs::Environment::with_prefix("ROCK_NODE").separator("__"))
        .build()?;
    let merged_config_value: toml::Value = settings.clone().try_deserialize()?;
    let config: RockConfig = settings.try_deserialize()?;
    let config = Arc::new(config);
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
        let mut providers = service_providers
            .write()
            .map_err(|e| anyhow::anyhow!("Failed to lock providers: {}", e))?;
        providers.insert(
            TypeId::of::<DatabaseManagerProvider>(),
            Arc::new(DatabaseManagerProvider::new(db_manager)),
        );
        info!("DatabaseManagerProvider registered as a core service provider.");
    }

    // --- Step 5: Build the AppContext ---
    let (tx_items, rx_items) = mpsc::channel::<events::BlockItemsReceived>(100);
    let (tx_verified, rx_verified) = mpsc::channel::<events::BlockVerified>(100);
    let (tx_verification_failed, _) = broadcast::channel::<events::BlockVerificationFailed>(100);
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
        tx_block_verification_failed: tx_verification_failed,
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

    // --- Step 7: Initialize Plugins ---
    info!("{}", "-".repeat(16));
    info!("Initializing plugins...");
    for plugin in &mut plugins {
        plugin.initialize(app_context.clone())?;
    }
    info!("{}", "-".repeat(16));

    // --- Step 8: - Dynamically Build and Spawn the Unified gRPC Server - ---
    let (grpc_shutdown_tx, mut grpc_shutdown_rx) = watch::channel(());
    let core_config = &app_context.config.core;
    let grpc_listen_address = format!("{}:{}", core_config.grpc_address, core_config.grpc_port);
    let grpc_socket_addr = grpc_listen_address.parse()?;

    let mut routes_builder = RoutesBuilder::default();
    let mut any_service_found = false;

    info!("Collecting gRPC services from enabled plugins...");
    for plugin in &mut plugins {
        if plugin.register_grpc_services(&mut routes_builder)? {
            info!("✔️  Found gRPC services in plugin: {}", plugin.name());
            any_service_found = true;
        }
    }

    if any_service_found {
        let routes = routes_builder.routes();

        tokio::spawn(async move {
            info!("Unified gRPC server listening on {}", grpc_socket_addr);

            if let Err(e) = Server::builder()
                .serve_with_shutdown(grpc_socket_addr, routes, async {
                    grpc_shutdown_rx.changed().await.ok();
                })
                .await
            {
                error!("Unified gRPC server failed: {}", e);
            }
        });
    } else {
        info!("No gRPC services were enabled; unified server will not be started.");
    }

    // --- Step 9: Start Plugin Background Tasks ---
    info!("{}", "-".repeat(16));
    info!("Starting plugin background tasks...");
    for plugin in &mut plugins {
        plugin.start()?;
    }
    info!("{}", "-".repeat(16));

    // --- Step 10: Report Status and Wait for Shutdown ---
    report_plugin_startup_status(&plugins);
    shutdown_signal().await;

    // --- Step 11: Graceful Shutdown ---
    info!("Shutdown signal received. Stopping services...");
    if grpc_shutdown_tx.send(()).is_err() {
        error!("Failed to send shutdown signal to unified gRPC server.");
    }

    for plugin in plugins.iter_mut().rev() {
        if plugin.is_running() {
            info!("Stopping plugin '{}'...", plugin.name());
            if let Err(e) = plugin.stop().await {
                error!("Error stopping plugin '{}': {}", plugin.name(), e);
            }
        }
    }

    tokio::time::sleep(Duration::from_millis(250)).await;
    info!("All plugins stopped. Exiting.");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_validate_config_file_exists_with_valid_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config_path = temp_dir.path().join("config.toml");
        std::fs::write(&config_path, "test = 'value'").unwrap();
        let result = validate_config_file_exists(&config_path);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_config_file_exists_with_missing_file() {
        let config_path = PathBuf::from("/nonexistent/path/config.toml");
        let result = validate_config_file_exists(&config_path);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Config file not found"));
    }

    #[test]
    fn test_format_plugin_list_empty() {
        let empty_list: Vec<String> = vec![];
        let result = format_plugin_list(&empty_list, "✅");
        assert_eq!(result, "None");
    }

    #[test]
    fn test_format_plugin_list_multiple_plugins() {
        let plugins = vec!["Plugin1".to_string(), "Plugin2".to_string()];
        let result = format_plugin_list(&plugins, "❌");
        let expected = "  ❌ Plugin1 - FAILED\n  ❌ Plugin2 - FAILED";
        assert_eq!(result, expected);
    }

    #[test]
    fn test_analyze_plugin_startup_results_all_successful() {
        let running_plugins = vec!["Plugin1".to_string(), "Plugin2".to_string()];
        let failed_plugins: Vec<String> = vec![];
        let (message, success) =
            analyze_plugin_startup_results(2, &running_plugins, &failed_plugins);
        assert!(success);
        assert!(message.contains("All 2 plugins started successfully"));
    }

    #[test]
    fn test_analyze_plugin_startup_results_partial_failure() {
        let running_plugins = vec!["Plugin1".to_string()];
        let failed_plugins = vec!["Plugin2".to_string(), "Plugin3".to_string()];
        let (message, success) =
            analyze_plugin_startup_results(3, &running_plugins, &failed_plugins);
        assert!(!success);
        assert!(message.contains("Some plugins failed to start"));
        assert!(message.contains("❌ Plugin2 - FAILED"));
        assert!(message.contains("✅ Plugin1 - OK"));
    }
}
