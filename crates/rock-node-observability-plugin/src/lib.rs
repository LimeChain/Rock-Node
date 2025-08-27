use async_trait::async_trait;
use axum::{
    body::Body,
    extract::State,
    http::{Response, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};
use rock_node_core::{
    app_context::AppContext, error::Result, plugin::Plugin, Error as CoreError, MetricsRegistry,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::watch;
use tracing::{error, info};

//================================================================================//
//=============================== UNIT TESTS =====================================//
//================================================================================//

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::Request;
    use rock_node_core::{
        app_context::AppContext,
        config::{
            BackfillConfig, BlockAccessServiceConfig, Config, CoreConfig, PluginConfigs,
            ServerStatusServiceConfig,
        },
        metrics::MetricsRegistry,
    };
    use std::{
        any::TypeId,
        collections::HashMap,
        sync::{Arc, RwLock},
        time::Duration,
    };
    use tokio::time::sleep;
    use tower::Service;

    fn create_test_context(enabled: bool) -> AppContext {
        let config = Config {
            plugins: PluginConfigs {
                observability: rock_node_core::config::ObservabilityConfig {
                    enabled,
                    listen_address: "127.0.0.1:0".to_string(), // Use port 0 for random port
                },
                backfill: BackfillConfig::default(),
                server_status_service: ServerStatusServiceConfig::default(),
                block_access_service: BlockAccessServiceConfig::default(),
                persistence_service: Default::default(),
                publish_service: Default::default(),
                verification_service: Default::default(),
                state_management_service: Default::default(),
                subscriber_service: Default::default(),
                query_service: Default::default(),
            },
            core: CoreConfig::default(),
        };

        let providers: HashMap<TypeId, Arc<dyn std::any::Any + Send + Sync>> = HashMap::new();

        AppContext {
            config: Arc::new(config),
            service_providers: Arc::new(RwLock::new(providers)),
            metrics: Arc::new(MetricsRegistry::new().unwrap()),
            capability_registry: Arc::new(Default::default()),
            block_data_cache: Arc::new(Default::default()),
            tx_block_items_received: tokio::sync::mpsc::channel(100).0,
            tx_block_verified: tokio::sync::mpsc::channel(100).0,
            tx_block_persisted: tokio::sync::broadcast::channel(100).0,
        }
    }

    #[test]
    fn test_plugin_name() {
        let plugin = ObservabilityPlugin::new();
        assert_eq!(plugin.name(), "observability-plugin");
    }

    #[test]
    fn test_plugin_initialization() {
        let mut plugin = ObservabilityPlugin::new();
        let context = create_test_context(true);

        let result = plugin.initialize(context);
        assert!(result.is_ok());
        assert!(plugin.context.is_some());
        assert!(!plugin.is_running());
    }

    #[test]
    fn test_plugin_initialization_twice() {
        let mut plugin = ObservabilityPlugin::new();
        let context = create_test_context(true);

        // First initialization should succeed
        assert!(plugin.initialize(context.clone()).is_ok());

        // Second initialization should still work
        assert!(plugin.initialize(context).is_ok());
    }

    #[test]
    fn test_start_without_initialization() {
        let mut plugin = ObservabilityPlugin::new();

        // Starting without initialization should fail
        let result = plugin.start();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("ObservabilityPlugin not initialized"));
    }

    #[tokio::test]
    async fn test_start_disabled_plugin() {
        let mut plugin = ObservabilityPlugin::new();
        let context = create_test_context(false); // Disabled

        plugin.initialize(context).unwrap();
        let result = plugin.start();
        assert!(result.is_ok()); // Should succeed but not actually start
        assert!(!plugin.is_running());
    }

    #[tokio::test]
    async fn test_start_enabled_plugin() {
        let mut plugin = ObservabilityPlugin::new();
        let context = create_test_context(true); // Enabled

        plugin.initialize(context).unwrap();
        let result = plugin.start();
        assert!(result.is_ok());
        assert!(plugin.is_running());

        // Give the server a moment to start
        sleep(Duration::from_millis(100)).await;

        // Clean shutdown
        plugin.stop().await.unwrap();
        assert!(!plugin.is_running());
    }

    #[tokio::test]
    async fn test_stop_without_start() {
        let mut plugin = ObservabilityPlugin::new();
        let context = create_test_context(true);

        plugin.initialize(context).unwrap();

        // Stopping without starting should be fine
        let result = plugin.stop().await;
        assert!(result.is_ok());
        assert!(!plugin.is_running());
    }

    #[tokio::test]
    async fn test_double_stop() {
        let mut plugin = ObservabilityPlugin::new();
        let context = create_test_context(true);

        plugin.initialize(context).unwrap();
        plugin.start().unwrap();
        sleep(Duration::from_millis(100)).await;

        // First stop should succeed
        assert!(plugin.stop().await.is_ok());
        assert!(!plugin.is_running());

        // Second stop should also succeed
        assert!(plugin.stop().await.is_ok());
        assert!(!plugin.is_running());
    }

    #[tokio::test]
    async fn test_plugin_restart() {
        let mut plugin = ObservabilityPlugin::new();
        let context = create_test_context(true);

        plugin.initialize(context).unwrap();

        // Start the plugin
        plugin.start().unwrap();
        assert!(plugin.is_running());
        sleep(Duration::from_millis(100)).await;

        // Stop it
        plugin.stop().await.unwrap();
        assert!(!plugin.is_running());
        sleep(Duration::from_millis(100)).await;

        // Start it again
        plugin.start().unwrap();
        assert!(plugin.is_running());
        sleep(Duration::from_millis(100)).await;

        // Final cleanup
        plugin.stop().await.unwrap();
        assert!(!plugin.is_running());
    }

    #[test]
    fn test_plugin_not_running_by_default() {
        let plugin = ObservabilityPlugin::new();
        assert!(!plugin.is_running());
    }

    #[tokio::test]
    async fn test_graceful_shutdown() {
        let mut plugin = ObservabilityPlugin::new();
        let context = create_test_context(true);

        plugin.initialize(context).unwrap();
        plugin.start().unwrap();
        assert!(plugin.is_running());

        // Give the server time to start
        sleep(Duration::from_millis(200)).await;

        // Request shutdown
        let stop_result = plugin.stop().await;
        assert!(stop_result.is_ok());

        // Give the server time to shutdown gracefully
        sleep(Duration::from_millis(200)).await;
        assert!(!plugin.is_running());
    }

    #[test]
    fn test_default_plugin_state() {
        let plugin = ObservabilityPlugin::new();
        assert!(plugin.context.is_none());
        assert!(!plugin.is_running());
        assert!(plugin.shutdown_tx.is_none());
    }

    // Test the HTTP handlers directly
    #[tokio::test]
    async fn test_health_check_handler() {
        let request = Request::get("/livez").body(Body::empty()).unwrap();
        let response = health_check().await;

        match response {
            axum::response::Response::Default(_) => {
                // This is the actual response type, but we can't easily inspect it
                // In a real scenario, we'd use a test client to make HTTP requests
            }
            _ => panic!("Unexpected response type"),
        }
    }

    #[tokio::test]
    async fn test_get_metrics_handler() {
        let metrics = Arc::new(MetricsRegistry::new().unwrap());
        let request = Request::get("/metrics").body(Body::empty()).unwrap();

        // Create a simple test by calling the handler function
        let response = get_metrics(State(metrics)).await;

        match response {
            axum::response::Response::Default(_) => {
                // Response is created successfully
            }
            _ => panic!("Unexpected response type"),
        }
    }

    #[tokio::test]
    async fn test_metrics_gathering() {
        let metrics = MetricsRegistry::new().unwrap();
        let gathered = metrics.gather();

        // Basic checks on the gathered metrics
        assert!(!gathered.is_empty(), "Metrics should not be empty");
        let metrics_str = String::from_utf8(gathered).unwrap();

        // Check that it contains Prometheus format headers
        assert!(
            metrics_str.contains("# TYPE")
                || metrics_str.contains("# HELP")
                || metrics_str.is_empty(),
            "Metrics should be in Prometheus format or empty"
        );
    }

    #[tokio::test]
    async fn test_server_bind_failure() {
        let mut plugin = ObservabilityPlugin::new();

        // Create context with an invalid address that will cause bind to fail
        let config = Config {
            plugins: PluginConfigs {
                observability: rock_node_core::config::ObservabilityConfig {
                    enabled: true,
                    listen_address: "invalid:address:99999".to_string(),
                },
                backfill: BackfillConfig::default(),
                server_status_service: ServerStatusServiceConfig::default(),
                block_access_service: BlockAccessServiceConfig::default(),
                persistence_service: Default::default(),
                publish_service: Default::default(),
                verification_service: Default::default(),
                state_management_service: Default::default(),
                subscriber_service: Default::default(),
                query_service: Default::default(),
            },
            core: CoreConfig::default(),
        };

        let providers: HashMap<TypeId, Arc<dyn std::any::Any + Send + Sync>> = HashMap::new();

        let context = AppContext {
            config: Arc::new(config),
            service_providers: Arc::new(RwLock::new(providers)),
            metrics: Arc::new(MetricsRegistry::new().unwrap()),
            capability_registry: Arc::new(Default::default()),
            block_data_cache: Arc::new(Default::default()),
            tx_block_items_received: tokio::sync::mpsc::channel(100).0,
            tx_block_verified: tokio::sync::mpsc::channel(100).0,
            tx_block_persisted: tokio::sync::broadcast::channel(100).0,
        };

        plugin.initialize(context).unwrap();

        // This should fail due to invalid address
        let result = plugin.start();
        assert!(result.is_err());
        assert!(!plugin.is_running());
    }

    #[tokio::test]
    async fn test_server_port_allocation() {
        let mut plugin = ObservabilityPlugin::new();
        let context = create_test_context(true); // Uses port 0 for auto-assignment

        plugin.initialize(context).unwrap();
        let result = plugin.start();
        assert!(result.is_ok());
        assert!(plugin.is_running());

        // Give the server time to start and bind to a port
        sleep(Duration::from_millis(200)).await;

        // Clean shutdown
        plugin.stop().await.unwrap();
        assert!(!plugin.is_running());
    }
}

#[derive(Debug, Default)]
pub struct ObservabilityPlugin {
    context: Option<AppContext>,
    running: Arc<AtomicBool>,
    shutdown_tx: Option<watch::Sender<()>>,
}

impl ObservabilityPlugin {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Axum handler that serves the Prometheus metrics.
async fn get_metrics(State(metrics): State<Arc<MetricsRegistry>>) -> impl IntoResponse {
    match Response::builder()
        .status(200)
        .header("Content-Type", prometheus::TEXT_FORMAT)
        .body(Body::from(metrics.gather()))
    {
        Ok(response) => response,
        Err(e) => {
            error!("Failed to build metrics response: {}", e);
            Response::builder()
                .status(500)
                .body(Body::from("Internal Server Error"))
                .unwrap_or_else(|_| Response::new(Body::from("Fatal Error")))
        }
    }
}

/// A simple health check handler that returns "OK".
async fn health_check() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

#[async_trait]
impl Plugin for ObservabilityPlugin {
    fn name(&self) -> &'static str {
        "observability-plugin"
    }

    fn initialize(&mut self, context: AppContext) -> Result<()> {
        info!("ObservabilityPlugin initialized.");
        self.context = Some(context);
        self.running = Arc::new(AtomicBool::new(false));
        Ok(())
    }

    fn start(&mut self) -> Result<()> {
        let context = self
            .context
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("ObservabilityPlugin not initialized"))?
            .clone();
        let config = &context.config.plugins.observability;
        if !config.enabled {
            info!("ObservabilityPlugin is disabled. Skipping start.");
            return Ok(());
        }

        info!("Starting Observability HTTP server...");

        let app = Router::new()
            .route("/livez", get(health_check))
            .route("/metrics", get(get_metrics))
            .with_state(context.metrics);

        let listen_address = config.listen_address.clone();
        let socket_addr: std::net::SocketAddr = listen_address.parse().map_err(|e| {
            anyhow::anyhow!(
                "Invalid observability listen address '{}': {}",
                listen_address,
                e
            )
        })?;

        let (shutdown_tx, mut shutdown_rx) = watch::channel(());
        self.shutdown_tx = Some(shutdown_tx);
        let running_clone = self.running.clone();

        self.running.store(true, Ordering::SeqCst);
        tokio::spawn(async move {
            let listener = match tokio::net::TcpListener::bind(&socket_addr).await {
                Ok(listener) => {
                    info!(
                        "Observability server listening on http://{}",
                        listen_address
                    );
                    listener
                }
                Err(e) => {
                    error!(
                        "Failed to bind observability server to {}: {}",
                        listen_address, e
                    );
                    running_clone.store(false, Ordering::SeqCst);
                    return;
                }
            };

            let server_future = axum::serve(listener, app.into_make_service())
                .with_graceful_shutdown(async move {
                    shutdown_rx.changed().await.ok();
                    info!("Gracefully shutting down Observability server...");
                });

            if let Err(e) = server_future.await {
                error!("Observability server failed: {}", e);
            }
            running_clone.store(false, Ordering::SeqCst);
        });

        Ok(())
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    async fn stop(&mut self) -> Result<()> {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            if shutdown_tx.send(()).is_err() {
                let msg =
                    "Failed to send shutdown signal to Observability server: receiver dropped.";
                error!("{}", msg);
                return Err(CoreError::PluginShutdown(msg.to_string()));
            }
        }
        self.running.store(false, Ordering::SeqCst);
        Ok(())
    }
}
