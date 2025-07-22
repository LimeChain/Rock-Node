use async_trait::async_trait;
use rock_node_core::{
    app_context::AppContext, error::Result, plugin::Plugin, BlockReaderProvider, Error as CoreError,
};
use rock_node_protobufs::org::hiero::block::api::block_node_service_server::BlockNodeServiceServer;
use service::StatusServiceImpl;
use std::{
    any::TypeId,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::sync::watch;
use tracing::{error, info, warn};

mod service;

#[derive(Debug, Default)]
pub struct StatusPlugin {
    context: Option<AppContext>,
    running: Arc<AtomicBool>,
    shutdown_tx: Option<watch::Sender<()>>,
}

impl StatusPlugin {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Plugin for StatusPlugin {
    fn name(&self) -> &'static str {
        "status-plugin"
    }

    fn initialize(&mut self, context: AppContext) -> Result<()> {
        info!("StatusPlugin initialized.");
        self.context = Some(context);
        self.running = Arc::new(AtomicBool::new(false));
        Ok(())
    }

    fn start(&mut self) -> Result<()> {
        info!("Starting Server Status Plugin...");
        let context = self
            .context
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("StatusPlugin not initialized"))?
            .clone();

        let config = &context.config.plugins.server_status_service;
        if !config.enabled {
            info!("Server StatusPlugin is disabled. Skipping start.");
            return Ok(());
        }

        let block_reader = {
            let providers = context
                .service_providers
                .read()
                .map_err(|_| anyhow::anyhow!("Failed to acquire read lock on service providers"))?;
            let key = TypeId::of::<BlockReaderProvider>();
            if let Some(provider_any) = providers.get(&key) {
                provider_any
                    .downcast_ref::<BlockReaderProvider>()
                    .map(|p| p.get_service())
                    .ok_or_else(|| {
                        anyhow::anyhow!("FATAL: Failed to downcast BlockReaderProvider.")
                    })
            } else {
                warn!("BlockReaderProvider not found. Status service will not be able to serve blocks.");
                return Ok(());
            }?
        };

        let listen_address = format!("{}:{}", config.grpc_address, config.grpc_port);
        let socket_addr = listen_address.parse().map_err(|e| {
            anyhow::anyhow!(
                "Failed to parse gRPC listen address '{}': {}",
                listen_address,
                e
            )
        })?;

        let service = StatusServiceImpl {
            block_reader,
            metrics: context.metrics.clone(),
        };
        let server = BlockNodeServiceServer::new(service);

        let (shutdown_tx, mut shutdown_rx) = watch::channel(());
        self.shutdown_tx = Some(shutdown_tx);
        let running_clone = self.running.clone();

        self.running.store(true, Ordering::SeqCst);
        tokio::spawn(async move {
            info!("Status gRPC service listening on {}", socket_addr);

            let server_future = tonic::transport::Server::builder()
                .add_service(server)
                .serve_with_shutdown(socket_addr, async move {
                    shutdown_rx.changed().await.ok();
                    info!("Gracefully shutting down Status gRPC server...");
                });

            if let Err(e) = server_future.await {
                error!("Status gRPC server failed: {}", e);
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
                let msg = "Failed to send shutdown signal to Status gRPC server: receiver dropped.";
                error!("{}", msg);
                return Err(CoreError::PluginShutdown(msg.to_string()));
            }
        }
        self.running.store(false, Ordering::SeqCst);
        Ok(())
    }
}
