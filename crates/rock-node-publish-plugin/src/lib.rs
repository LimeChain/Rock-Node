use async_trait::async_trait;
use rock_node_core::{
    app_context::AppContext, error::Result, plugin::Plugin, BlockReaderProvider,
};
use rock_node_protobufs::org::hiero::block::api::{
    block_stream_publish_service_server::BlockStreamPublishServiceServer,
    publish_stream_response::{self, end_of_stream},
    PublishStreamResponse,
};
use service::PublishServiceImpl;
use state::SharedState;
use std::{
    any::TypeId,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::watch;
use tracing::{debug, error, info, warn};

mod service;
mod session_manager;
mod state;

#[derive(Debug, Default)]
pub struct PublishPlugin {
    context: Option<AppContext>,
    shared_state: Option<Arc<SharedState>>,
    running: Arc<AtomicBool>,
    shutdown_tx: Option<watch::Sender<()>>,
}

impl PublishPlugin {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Plugin for PublishPlugin {
    fn name(&self) -> &'static str {
        "publish-plugin"
    }

    fn initialize(&mut self, context: AppContext) -> Result<()> {
        self.context = Some(context);
        self.running = Arc::new(AtomicBool::new(false));
        info!("PublishPlugin initialized.");
        Ok(())
    }

    fn start(&mut self) -> Result<()> {
        info!("Starting PublishPlugin gRPC Server...");
        let context = self
            .context
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("PublishPlugin not initialized"))?
            .clone();

        let config = &context.config.plugins.publish_service;
        if !config.enabled {
            info!("PublishPlugin is disabled. Skipping start.");
            return Ok(());
        }
        let listen_address = format!("{}:{}", config.grpc_address, config.grpc_port);

        let shared_state = Arc::new(SharedState::new());
        self.shared_state = Some(shared_state.clone());
        {
            let providers = context
                .service_providers
                .read()
                .map_err(|e| anyhow::anyhow!("Failed to acquire read lock on service providers: {}", e))?;
            
            if let Some(provider_any) = providers.get(&TypeId::of::<BlockReaderProvider>()) {
                if let Some(provider_handle) = provider_any.downcast_ref::<BlockReaderProvider>() {
                    let block_reader = provider_handle.get_service();

                    let block_number_for_state = match block_reader.get_latest_persisted_block_number() {
                        Ok(Some(num)) => num as i64,
                        Ok(None) => -1,
                        Err(e) => {
                            warn!("Could not get latest persisted block on startup: {}. Defaulting to -1.", e);
                            -1
                        }
                    };

                    shared_state.set_latest_persisted_block(block_number_for_state);
                    debug!("Initialized PublishPlugin state. Latest persisted block is: {}", block_number_for_state);
                } else {
                     error!("FATAL: Failed to downcast BlockReaderProvider. This indicates a critical type mismatch bug.");
                }
            } else {
                warn!("No BlockReaderProvider handle found. Is the persistence plugin running? Defaulting latest block to -1.");
                shared_state.set_latest_persisted_block(-1);
            }
        }

        let service = PublishServiceImpl {
            context: context.clone(),
            shared_state,
        };

        const MAX_MESSAGE_SIZE: usize = 1024 * 1024 * 32;
        let server = BlockStreamPublishServiceServer::new(service)
            .max_decoding_message_size(MAX_MESSAGE_SIZE);

        let socket_addr = listen_address.parse().map_err(|e| {
            anyhow::anyhow!(
                "Failed to parse gRPC listen address '{}': {}",
                listen_address,
                e
            )
        })?;

        let (shutdown_tx, mut shutdown_rx) = watch::channel(());
        self.shutdown_tx = Some(shutdown_tx);
        let running_clone = self.running.clone();

        self.running.store(true, Ordering::SeqCst);
        tokio::spawn(async move {
            info!("Publish gRPC service listening on {}", socket_addr);

            let server_future = tonic::transport::Server::builder()
                .http2_keepalive_interval(Some(Duration::from_secs(30)))
                .http2_keepalive_timeout(Some(Duration::from_secs(10)))
                .tcp_nodelay(true)
                .add_service(server)
                .serve_with_shutdown(socket_addr, async move {
                    shutdown_rx.changed().await.ok();
                    info!("Gracefully shutting down Publish gRPC server...");
                });

            if let Err(e) = server_future.await {
                error!("Publish gRPC server failed: {}", e);
            }
            running_clone.store(false, Ordering::SeqCst);
        });

        Ok(())
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    async fn stop(&mut self) -> Result<()> {
        if !self.is_running() {
            return Ok(());
        }
        
        info!("Stopping PublishPlugin...");

        if let Some(shared_state) = &self.shared_state {
            let latest_block = shared_state.get_latest_persisted_block();
            let block_number_to_send = if latest_block < 0 { 0 } else { latest_block as u64 };

            let end_stream_msg = PublishStreamResponse {
                response: Some(publish_stream_response::Response::EndStream(
                    publish_stream_response::EndOfStream {
                        status: end_of_stream::Code::InternalError as i32,
                        block_number: block_number_to_send,
                    },
                )),
            };

            info!("Sending EndStream to {} active clients.", shared_state.active_sessions.len());
            for entry in shared_state.active_sessions.iter() {
                let _ = entry.value().send(Ok(end_stream_msg.clone())).await;
            }
        }
        
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }

        tokio::time::sleep(Duration::from_millis(100)).await;

        self.running.store(false, Ordering::SeqCst);
        info!("PublishPlugin stopped.");
        Ok(())
    }
}
