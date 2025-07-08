use rock_node_core::{
    app_context::AppContext, block_reader::BlockReader, error::Result, plugin::Plugin,
    BlockReaderProvider,
};
use rock_node_protobufs::org::hiero::block::api::block_stream_publish_service_server::BlockStreamPublishServiceServer;
use state::SharedState;
use std::any::TypeId;
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info, warn};

mod service;
mod session_manager;
mod state;

use service::PublishServiceImpl;

#[derive(Debug, Default)]
pub struct PublishPlugin {
    context: Option<AppContext>,
}

impl PublishPlugin {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Plugin for PublishPlugin {
    fn name(&self) -> &'static str {
        "publish-plugin"
    }

    fn initialize(&mut self, context: AppContext) -> Result<()> {
        self.context = Some(context);
        info!("PublishPlugin initialized.");
        Ok(())
    }

    fn start(&mut self) -> Result<()> {
        info!("Starting PublishPlugin gRPC Server...");
        let context = self.context.as_ref().unwrap().clone();

        let config = &context.config.plugins.publish_service;
        if !config.enabled {
            info!("PublishPlugin is disabled. Skipping start.");
            return Ok(());
        }
        let listen_address = format!("{}:{}", config.grpc_address, config.grpc_port);

        let shared_state = Arc::new(SharedState::new());
        {
            let providers = context.service_providers.read().unwrap();
            let key = TypeId::of::<BlockReaderProvider>();
            if let Some(provider_any) = providers.get(&key) {
                if let Some(provider_handle) = provider_any.downcast_ref::<BlockReaderProvider>() {
                    let block_reader: Arc<dyn BlockReader> = provider_handle.get_service();

                    let block_number_for_state = match block_reader
                        .get_latest_persisted_block_number()
                    {
                        Ok(Some(num)) => num as i64,
                        Ok(None) => -1, // Use -1 as the sentinel for "no blocks" in our state
                        Err(e) => {
                            error!("Could not get latest persisted block on startup: {}. Defaulting to -1.", e);
                            -1
                        }
                    };

                    shared_state.set_latest_persisted_block(block_number_for_state);

                    info!(
                        "Successfully retrieved BlockReader. Latest persisted block is: {}",
                        block_number_for_state
                    );
                } else {
                    warn!("Found BlockReaderProvider key, but failed to downcast. This indicates a critical type mismatch bug.");
                }
            } else {
                warn!("No BlockReaderProvider handle found. Is the persistence plugin configured and running correctly?");
            }
        }

        let service = PublishServiceImpl {
            context: context.clone(),
            shared_state,
        };

        const MAX_MESSAGE_SIZE: usize = 1024 * 1024 * 32; // 32 MB
        let server = BlockStreamPublishServiceServer::new(service)
            .max_decoding_message_size(MAX_MESSAGE_SIZE);

        tokio::spawn(async move {
            info!("Publish gRPC service listening on {}", listen_address);

            if let Err(e) = tonic::transport::Server::builder()
                .http2_keepalive_interval(Some(Duration::from_secs(30)))
                .http2_keepalive_timeout(Some(Duration::from_secs(10)))
                .tcp_nodelay(true)
                .add_service(server)
                .serve(listen_address.parse().unwrap())
                .await
            {
                tracing::error!("gRPC server failed: {}", e);
            }
        });

        Ok(())
    }
}
