use anyhow::Context;
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
use tracing::info;

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
async fn get_metrics(State(metrics): State<Arc<MetricsRegistry>>) -> Response<Body> {
    Response::builder()
        .status(200)
        .header("Content-Type", prometheus::TEXT_FORMAT)
        .body(Body::from(metrics.gather()))
        .unwrap()
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
        let context = self.context.as_ref().unwrap().clone();
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

        tokio::spawn(async move {
            info!(
                "Observability server listening on http://{}",
                listen_address
            );
            let listener = tokio::net::TcpListener::bind(&listen_address)
                .await
                .with_context(|| {
                    format!("Failed to bind observability server to {}", listen_address)
                })
                .unwrap();

            axum::serve(listener, app.into_make_service())
                .await
                .unwrap();
        });

        Ok(())
    }
}
