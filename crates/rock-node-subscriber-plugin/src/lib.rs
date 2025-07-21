use crate::service::SubscriberServiceImpl;
use async_trait::async_trait;
use rock_node_core::{app_context::AppContext, error::Result as CoreResult, plugin::Plugin};
use rock_node_protobufs::org::hiero::block::api::block_stream_subscribe_service_server::BlockStreamSubscribeServiceServer;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::{watch, Notify};
use tracing::{error, info};

mod error;
mod service;
mod session;

pub struct SubscriberPlugin {
    context: Option<Arc<AppContext>>,
    running: Arc<AtomicBool>,
    shutdown_tx: Option<watch::Sender<()>>,
    service_shutdown_notify: Arc<Notify>,
}

impl SubscriberPlugin {
    pub fn new() -> Self {
        Self {
            context: None,
            running: Arc::new(AtomicBool::new(false)),
            shutdown_tx: None,
            service_shutdown_notify: Arc::new(Notify::new()),
        }
    }
}

impl Default for SubscriberPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Plugin for SubscriberPlugin {
    fn name(&self) -> &'static str {
        "subscriber-plugin"
    }

    fn initialize(&mut self, context: AppContext) -> CoreResult<()> {
        self.context = Some(Arc::new(context));
        info!("SubscriberPlugin initialized.");
        Ok(())
    }

    fn start(&mut self) -> CoreResult<()> {
        info!("Starting SubscriberPlugin gRPC Server...");
        let context = self
            .context
            .as_ref()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("SubscriberPlugin not initialized"))?;

        let config = &context.config.plugins.subscriber_service;
        if !config.enabled {
            info!("SubscriberPlugin is disabled. Skipping start.");
            return Ok(());
        }
        let listen_address = format!("{}:{}", config.grpc_address, config.grpc_port);
        let socket_addr: std::net::SocketAddr = listen_address.parse().map_err(|e| {
            anyhow::anyhow!(
                "Failed to parse gRPC listen address '{}': {}",
                listen_address,
                e
            )
        })?;

        let service = SubscriberServiceImpl::new(context, self.service_shutdown_notify.clone());
        let server = BlockStreamSubscribeServiceServer::new(service);

        let (shutdown_tx, mut shutdown_rx) = watch::channel(());
        self.shutdown_tx = Some(shutdown_tx);
        let running_clone = self.running.clone();

        self.running.store(true, Ordering::SeqCst);
        tokio::spawn(async move {
            info!("Subscriber gRPC service listening on {}", socket_addr);

            let server_future = tonic::transport::Server::builder()
                .add_service(server)
                .serve_with_shutdown(socket_addr, async move {
                    shutdown_rx.changed().await.ok();
                    info!("Gracefully shutting down Subscriber gRPC server...");
                });

            if let Err(e) = server_future.await {
                error!("Subscriber gRPC server failed: {}", e);
            }
            running_clone.store(false, Ordering::SeqCst);
        });

        Ok(())
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    async fn stop(&mut self) -> CoreResult<()> {
        info!("Stopping SubscriberPlugin...");
        // Notify all active session tasks to shut down.
        self.service_shutdown_notify.notify_waiters();

        // Signal the main gRPC server to stop accepting new connections.
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        self.running.store(false, Ordering::SeqCst);
        Ok(())
    }
}
