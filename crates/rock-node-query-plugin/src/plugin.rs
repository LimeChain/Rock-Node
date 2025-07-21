use crate::service::CryptoServiceImpl;
use async_trait::async_trait;
use rock_node_core::{
    app_context::AppContext, error::Result as CoreResult, plugin::Plugin,
    state_reader::StateReaderProvider,
};
use rock_node_protobufs::proto::crypto_service_server::CryptoServiceServer;
use std::{
    any::TypeId,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::sync::watch;
use tracing::{error, info, warn};

/// The main plugin struct that registers and runs the gRPC query services.
#[derive(Debug, Default)]
pub struct QueryPlugin {
    context: Option<AppContext>,
    running: Arc<AtomicBool>,
    shutdown_tx: Option<watch::Sender<()>>,
}

impl QueryPlugin {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Plugin for QueryPlugin {
    fn name(&self) -> &'static str {
        "rock-node-query-plugin"
    }

    fn initialize(&mut self, context: AppContext) -> CoreResult<()> {
        info!("QueryPlugin initialized.");
        self.context = Some(context);
        self.running = Arc::new(AtomicBool::new(false));
        Ok(())
    }

    fn start(&mut self) -> CoreResult<()> {
        let context = self
            .context
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("QueryPlugin not initialized"))?
            .clone();
        let config = &context.config.plugins.query_service;

        if !config.enabled {
            info!("QueryPlugin is disabled. Skipping start.");
            return Ok(());
        }

        let state_reader = {
            let providers = context
                .service_providers
                .read()
                .map_err(|_| anyhow::anyhow!("Failed to acquire read lock on service providers"))?;
            providers
                .get(&TypeId::of::<StateReaderProvider>())
                .and_then(|p| p.downcast_ref::<StateReaderProvider>())
                .map(|p| p.get_reader())
                .ok_or_else(|| {
                    warn!("StateReaderProvider not found. QueryPlugin cannot start.");
                    rock_node_core::Error::PluginInitialization(
                        "StateReaderProvider not found".to_string(),
                    )
                })?
        };

        let listen_address = format!("{}:{}", config.grpc_address, config.grpc_port);
        let socket_addr = listen_address.parse().map_err(|e| {
            anyhow::anyhow!(
                "Failed to parse gRPC listen address '{}': {}",
                listen_address,
                e
            )
        })?;

        let crypto_service = CryptoServiceImpl::new(state_reader);
        let server = CryptoServiceServer::new(crypto_service);

        let (shutdown_tx, mut shutdown_rx) = watch::channel(());
        self.shutdown_tx = Some(shutdown_tx);
        let running_clone = self.running.clone();

        self.running.store(true, Ordering::SeqCst);
        tokio::spawn(async move {
            info!(
                "QueryPlugin: CryptoService gRPC listening on {}",
                socket_addr
            );

            let server_future = tonic::transport::Server::builder()
                .add_service(server)
                .serve_with_shutdown(socket_addr, async move {
                    shutdown_rx.changed().await.ok();
                    info!("Gracefully shutting down Query gRPC server...");
                });

            if let Err(e) = server_future.await {
                error!("QueryPlugin gRPC server failed: {}", e);
            }
            running_clone.store(false, Ordering::SeqCst);
        });

        Ok(())
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    async fn stop(&mut self) -> CoreResult<()> {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            if shutdown_tx.send(()).is_err() {
                error!("Failed to send shutdown signal to Query gRPC server: receiver dropped.");
            }
        }
        self.running.store(false, Ordering::SeqCst);
        Ok(())
    }
}
