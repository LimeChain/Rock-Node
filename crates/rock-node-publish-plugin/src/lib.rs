use rock_node_core::{app_context::AppContext, error::Result, plugin::Plugin};
use rock_node_protobufs::org::hiero::block::api::block_stream_publish_service_server::BlockStreamPublishServiceServer;
use state::SharedState;
use std::any::TypeId;
use std::sync::Arc;
use tracing::{info, warn}; // <-- Import `warn` here

mod service;
mod session_manager;
mod state;

use service::PublishServiceImpl;

// A trait needed for the startup logic. It should live in `rock-node-core` eventually.
pub trait BlockReader: Send + Sync {
    fn get_latest_persisted_block_number(&self) -> i64;
}

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

    // Return the correct `Result` type as defined by the trait
    fn initialize(&mut self, context: AppContext) -> Result<()> {
        self.context = Some(context);
        info!("PublishPlugin initialized.");
        Ok(())
    }

    // Return the correct `Result` type as defined by the trait
    fn start(&mut self) -> Result<()> {
        info!("Starting PublishPlugin gRPC Server...");
        let context = self.context.as_ref().unwrap().clone();
        
        let config = &self.context.as_ref().unwrap().config.plugins.publish_service;
        if !config.enabled {
            info!("PublishPlugin is disabled. Skipping start.");
            return Ok(());
        }
        let grpc_address = config.grpc_address.clone();
        let grpc_port = config.grpc_port;
        let listen_address = format!("{}:{}", grpc_address, grpc_port);

        let shared_state = Arc::new(SharedState::new());

        // VERY TEMPORARY: Set the latest persisted block to -1 to start with a clean state.
        shared_state.set_latest_persisted_block(-1);

        let shared_state_clone = shared_state.clone();
        let context_clone = context.clone();
        tokio::spawn(async move {
            info!("Querying for latest persisted block number...");
            let providers = context_clone.service_providers.read().await;
            if let Some(provider) = providers.get(&TypeId::of::<Arc<dyn BlockReader>>()) {
                if let Some(block_reader) = provider.downcast_ref::<Arc<dyn BlockReader>>() {
                    let block_number = block_reader.get_latest_persisted_block_number();
                    shared_state_clone.set_latest_persisted_block(block_number);
                    info!("Set latest persisted block to: {}", block_number);
                }
            } else {
                warn!("No BlockReader provider found. Starting with clean state (block -1).");
            }
        });
        
        let service = PublishServiceImpl {
            context,
            shared_state,
        };
        let server = BlockStreamPublishServiceServer::new(service);

        tokio::spawn(async move {
            info!("Publish gRPC service listening on {}", listen_address);
            tonic::transport::Server::builder()
                .add_service(server)
                .serve(listen_address.parse().unwrap())
                .await
                // Use map_err to convert the error type to our core::Error
                .map_err(|e| rock_node_core::Error::Other(e.into()))
                .unwrap();
        });
        
        Ok(())
    }
}
