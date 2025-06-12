// File: rock-node-workspace/crates/rock-node-observability-plugin/src/lib.rs

use anyhow::Context;
use axum::{http::StatusCode, response::IntoResponse, routing::get, Router};
use rock_node_core::{app_context::AppContext, error::Result, plugin::Plugin};
use tracing::info;

#[derive(Debug, Default)]
pub struct ObservabilityPlugin {
    // This field will hold the context after initialization.
    context: Option<AppContext>,
}
impl ObservabilityPlugin {
    pub fn new() -> Self {
        Self::default()
    }
}

// A simple health check handler that returns "OK".
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

    fn start(&self) -> Result<()> {
        let config = &self.context.as_ref().unwrap().config.plugins.observability;
        if !config.enabled {
            info!("ObservabilityPlugin is disabled. Skipping start.");
            return Ok(());
        }

        info!("Starting Observability HTTP server...");

        // Create the axum router with our health check endpoint.
        let app = Router::new().route("/livez", get(health_check));

        // Read the full listen address from our config file.
        let listen_address = config.listen_address.clone();

        // Spawn a new asynchronous task to run the web server.
        // This is crucial - it must not block the main application thread.
        tokio::spawn(async move {
            info!("Observability server listening on http://{}", listen_address);

            // Bind the server to the configured address.
            let listener = tokio::net::TcpListener::bind(&listen_address)
                .await
                .with_context(|| format!("Failed to bind observability server to {}", listen_address))
                .unwrap(); // Using unwrap here is okay for a top-level task that should crash if it fails to start.

            axum::serve(listener, app).await.unwrap();
        });

        Ok(())
    }
}
