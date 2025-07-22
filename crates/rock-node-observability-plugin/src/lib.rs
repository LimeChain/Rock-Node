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
