use rock_node_core::{app_context::AppContext, block_reader::BlockReader, error::Result, plugin::Plugin};
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

        let providers = context.service_providers.read().unwrap();
        if let Some(provider) = providers.get(&TypeId::of::<Arc<dyn BlockReader>>()) {
            if let Some(block_reader) = provider.downcast_ref::<Arc<dyn BlockReader>>() {
                let block_number = block_reader.get_latest_persisted_block_number();
                shared_state.set_latest_persisted_block(block_number);
                info!("BlockReader service found. Initial latest persisted block is: {}", block_number);
            }
        } else {
            warn!("No BlockReader provider found. Ensure PersistencePlugin is initialized before PublishPlugin. Starting with clean state (block -1).");
            shared_state.set_latest_persisted_block(-1);
        }
        
        let service = PublishServiceImpl {
            context: context.clone(),
            shared_state,
        };
        let server = BlockStreamPublishServiceServer::new(service);

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
