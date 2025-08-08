use async_trait::async_trait;
use backfill_manager::BackfillManager;
use filter::FilterManager;
use rock_node_core::{
    app_context::AppContext, error::Result, events::BlockItemsReceived, plugin::Plugin,
    BlockReaderProvider,
};
use rock_node_protobufs::org::hiero::block::api::block_stream_publish_service_server::BlockStreamPublishServiceServer;
use service::IngressServiceImpl;
use state::SharedState;
use std::{
    any::TypeId,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::sync::{broadcast, watch, Notify};
use tracing::{error, info, warn};

mod backfill_manager;
mod consensus_manager;
mod filter;
mod service;
mod state;

#[derive(Debug, Default)]
pub struct IngressPlugin {
    context: Option<AppContext>,
    shared_state: Option<Arc<SharedState>>,
    running: Arc<AtomicBool>,
    shutdown_tx: Option<watch::Sender<()>>,
    task_shutdown_notify: Arc<Notify>,
}

impl IngressPlugin {
    pub fn new() -> Self {
        Self {
            task_shutdown_notify: Arc::new(Notify::new()),
            ..Default::default()
        }
    }
}

#[async_trait]
impl Plugin for IngressPlugin {
    fn name(&self) -> &'static str {
        "ingress-plugin"
    }

    fn initialize(&mut self, context: AppContext) -> Result<()> {
        self.context = Some(context);
        self.running = Arc::new(AtomicBool::new(false));
        info!("IngressPlugin initialized.");
        Ok(())
    }

    fn start(&mut self) -> Result<()> {
        let context = self
            .context
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("IngressPlugin not initialized"))?
            .clone();

        if !context.config.plugins.ingress_service.enabled {
            info!("IngressPlugin is disabled in configuration. Skipping start.");
            return Ok(());
        }

        info!("Starting IngressPlugin...");

        let config = &context.config.plugins.ingress_service;
        let listen_address = format!("{}:{}", config.grpc_address, config.grpc_port);

        let shared_state = Arc::new(SharedState::new());
        self.shared_state = Some(shared_state.clone());

        // --- Initialize Shared State ---
        {
            let providers = context.service_providers.read().map_err(|e| {
                anyhow::anyhow!("Failed to acquire read lock on service providers: {}", e)
            })?;
            if let Some(provider_any) = providers.get(&TypeId::of::<BlockReaderProvider>()) {
                if let Some(provider_handle) = provider_any.downcast_ref::<BlockReaderProvider>() {
                    let block_reader = provider_handle.get_reader();
                    let block_number_for_state = match block_reader
                        .get_latest_persisted_block_number()
                    {
                        Ok(Some(num)) => num as i64,
                        Ok(None) => (context.config.core.start_block_number as i64) - 1,
                        Err(e) => {
                            warn!("Could not get latest persisted block on startup: {}. Defaulting based on start_block_number.", e);
                            (context.config.core.start_block_number as i64) - 1
                        }
                    };
                    shared_state.set_latest_persisted_block(block_number_for_state);
                } else {
                    return Err(
                        anyhow::anyhow!("FATAL: Failed to downcast BlockReaderProvider.").into(),
                    );
                }
            } else {
                warn!("No BlockReaderProvider handle found. Defaulting latest block based on start_block_number.");
                shared_state.set_latest_persisted_block(
                    (context.config.core.start_block_number as i64) - 1,
                );
            }
        }

        // --- Internal Channel Setup ---
        let (internal_tx, _) = broadcast::channel::<BlockItemsReceived>(256);

        // --- Spawn Consensus Manager gRPC Server ---
        let service = IngressServiceImpl {
            context: context.clone(),
            shared_state: shared_state.clone(),
            internal_tx: internal_tx.clone(),
        };
        let server = BlockStreamPublishServiceServer::new(service);
        let socket_addr = listen_address.parse().map_err(|e| {
            anyhow::anyhow!(
                "Failed to parse gRPC listen address '{}': {}",
                listen_address,
                e
            )
        })?;
        let (shutdown_tx, mut shutdown_rx) = watch::channel(());
        self.shutdown_tx = Some(shutdown_tx);

        tokio::spawn(async move {
            info!("Ingress gRPC service listening on {}", socket_addr);
            let server_future = tonic::transport::Server::builder()
                .add_service(server)
                .serve_with_shutdown(socket_addr, async move {
                    shutdown_rx.changed().await.ok();
                });
            if let Err(e) = server_future.await {
                error!("Ingress gRPC server failed: {}", e);
            }
        });

        // --- Spawn Backfill Manager ---
        if config.backfill.enabled {
            if config.backfill.peers.is_empty() {
                warn!("Backfill is enabled, but no peers are configured. Backfill manager will not start.");
            } else {
                info!("Starting Backfill Manager task...");
                let backfill_manager = BackfillManager::new(
                    context.clone(),
                    shared_state.clone(),
                    self.task_shutdown_notify.clone(),
                );
                tokio::spawn(async move {
                    backfill_manager.run().await;
                });
            }
        }

        // --- Spawn Coordinator and Filter Tasks ---
        let coordinator_rx = internal_tx.subscribe();
        let filter_rx = internal_tx.subscribe();

        let coordinator_context = context.clone();
        let coordinator_shutdown = self.task_shutdown_notify.clone();
        tokio::spawn(async move {
            info!("Ingress Coordinator task started.");
            let mut rx = coordinator_rx;
            loop {
                tokio::select! {
                    _ = coordinator_shutdown.notified() => break,
                    Ok(event) = rx.recv() => {
                        if coordinator_context.tx_block_items_received.send(event).await.is_err() {
                            warn!("Main pipeline channel closed. Coordinator stopping.");
                            break;
                        }
                    }
                }
            }
            info!("Ingress Coordinator task terminated.");
        });

        info!("Starting Filter Manager task...");
        let filter_manager = FilterManager::new(
            context.clone(),
            filter_rx,
            self.task_shutdown_notify.clone(),
        );
        tokio::spawn(async move {
            filter_manager.run().await;
        });

        self.running.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    async fn stop(&mut self) -> Result<()> {
        self.task_shutdown_notify.notify_waiters();
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
        self.running.store(false, Ordering::SeqCst);
        info!("IngressPlugin stopped.");
        Ok(())
    }
}
