use rock_node_core::{app_context::AppContext, block_reader::BlockReader, error::Result, plugin::Plugin, BlockReaderProvider};
use rock_node_protobufs::org::hiero::block::api::block_stream_publish_service_server::BlockStreamPublishServiceServer;
use state::SharedState;
use std::any::TypeId;
use std::sync::Arc;
use tracing::{info, warn};

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
                    
                    let block_number = block_reader.get_latest_persisted_block_number();
                    shared_state.set_latest_persisted_block(block_number);
                    info!(
                        "Successfully retrieved BlockReader via provider handle. Latest persisted block is: {}",
                        block_number
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
            shared_state, // Pass the initialized state
        };
        let server = BlockStreamPublishServiceServer::new(service);

        // Spawn the server task. It will now have the correct initial state.
        tokio::spawn(async move {
            info!("Publish gRPC service listening on {}", listen_address);
            if let Err(e) = tonic::transport::Server::builder()
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
