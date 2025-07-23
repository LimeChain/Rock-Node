use async_trait::async_trait;
use rock_node_core::{
    app_context::AppContext, error::Result, plugin::Plugin, BlockReaderProvider, Error as CoreError,
};
use rock_node_protobufs::org::hiero::block::api::block_access_service_server::BlockAccessServiceServer;
use service::BlockAccessServiceImpl;
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
pub struct BlockAccessPlugin {
    context: Option<AppContext>,
    running: Arc<AtomicBool>,
    shutdown_tx: Option<watch::Sender<()>>,
}

impl BlockAccessPlugin {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Plugin for BlockAccessPlugin {
    fn name(&self) -> &'static str {
        "block-access-plugin"
    }

    fn initialize(&mut self, context: AppContext) -> Result<()> {
        info!("BlockAccessPlugin initialized.");
        self.context = Some(context);
        self.running = Arc::new(AtomicBool::new(false));
        Ok(())
    }

    fn start(&mut self) -> Result<()> {
        info!("Starting BlockAccessPlugin...");
        let context = self
            .context
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("BlockAccessPlugin not initialized - initialize() must be called before start()"))?
            .clone();

        let config = &context.config.plugins.block_access_service;
        if !config.enabled {
            info!("BlockAccessPlugin is disabled. Skipping start.");
            return Ok(());
        }

        let block_reader = {
            let providers = context
                .service_providers
                .read()
                .map_err(|_| anyhow::anyhow!("Failed to acquire read lock on service providers"))?;
            let key = TypeId::of::<BlockReaderProvider>();

            if let Some(provider_any) = providers.get(&key) {
                if let Some(provider_handle) = provider_any.downcast_ref::<BlockReaderProvider>() {
                    info!("Successfully retrieved BlockReaderProvider handle.");
                    provider_handle.get_reader()
                } else {
                    return Err(
                        anyhow::anyhow!("FATAL: Failed to downcast BlockReaderProvider.").into(),
                    );
                }
            } else {
                warn!(
                    "BlockReaderProvider not found. The service will not be able to serve blocks."
                );
                return Ok(());
            }
        };

        let listen_address = format!("{}:{}", config.grpc_address, config.grpc_port);

        let service = BlockAccessServiceImpl {
            block_reader,
            metrics: context.metrics.clone(),
        };
        let server = BlockAccessServiceServer::new(service);

        // Parse the address before spawning to handle errors properly
        let socket_addr = listen_address.parse().map_err(|e| {
            anyhow::anyhow!(
                "Failed to parse gRPC listen address '{}': {}",
                listen_address,
                e
            )
        })?;

        // Create a channel for shutdown signaling
        let (shutdown_tx, mut shutdown_rx) = watch::channel(());
        self.shutdown_tx = Some(shutdown_tx);
        let running_clone = self.running.clone();

        self.running.store(true, Ordering::SeqCst);
        tokio::spawn(async move {
            info!("BlockAccess gRPC service listening on {}", socket_addr);

            let server_future = tonic::transport::Server::builder()
                .add_service(server)
                .serve_with_shutdown(socket_addr, async move {
                    shutdown_rx.changed().await.ok();
                    info!("Gracefully shutting down BlockAccess gRPC server...");
                });

            if let Err(e) = server_future.await {
                error!("BlockAccess gRPC server failed: {}", e);
            }

            // Server has shut down, update the running status
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
                    "Failed to send shutdown signal to BlockAccess gRPC server: receiver dropped.";
                error!("{}", msg);
                return Err(CoreError::PluginShutdown(msg.to_string()));
            }
        }
        self.running.store(false, Ordering::SeqCst);
        Ok(())
    }
}
