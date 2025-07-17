use axum::{
    body::Body,
    extract::State,
    http::{Response, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};
use rock_node_core::{app_context::AppContext, error::Result, plugin::Plugin, MetricsRegistry};
use std::sync::Arc;
use tracing::{error, info};

#[derive(Debug, Default)]
pub struct ObservabilityPlugin {
    context: Option<AppContext>,
}

impl ObservabilityPlugin {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Axum handler that serves the Prometheus metrics.
/// It takes the shared `MetricsRegistry` from the application state.
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

impl Plugin for ObservabilityPlugin {
    fn name(&self) -> &'static str {
        "observability-plugin"
    }

    fn initialize(&mut self, context: AppContext) -> Result<()> {
        info!("ObservabilityPlugin initialized.");
        self.context = Some(context);
        Ok(())
    }

    fn start(&mut self) -> Result<()> {
        let context = self.context
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("ObservabilityPlugin not initialized - initialize() must be called before start()"))?
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

        // Validate the address format before spawning
        let _socket_addr: std::net::SocketAddr = listen_address.parse().map_err(|e| {
            anyhow::anyhow!(
                "Invalid observability listen address '{}': {}",
                listen_address,
                e
            )
        })?;

        tokio::spawn(async move {
            let listener = match tokio::net::TcpListener::bind(&listen_address).await {
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
                    return;
                }
            };

            if let Err(e) = axum::serve(listener, app.into_make_service()).await {
                error!("Observability server failed: {}", e);
            }
        });

        Ok(())
    }
}
