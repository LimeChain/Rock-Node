use rock_node_core::{
    app_context::AppContext, error::Result, plugin::Plugin, BlockReaderProvider,
};
use rock_node_protobufs::org::hiero::block::api::block_access_service_server::BlockAccessServiceServer;
use std::any::TypeId;
use tracing::{error, info, warn};

mod service;
use service::BlockAccessServiceImpl;

#[derive(Debug, Default)]
pub struct BlockAccessPlugin {
    context: Option<AppContext>,
}

impl BlockAccessPlugin {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Plugin for BlockAccessPlugin {
    fn name(&self) -> &'static str {
        "block-access-plugin"
    }

    fn initialize(&mut self, context: AppContext) -> Result<()> {
        info!("BlockAccessPlugin initialized.");
        self.context = Some(context);
        Ok(())
    }

    fn start(&mut self) -> Result<()> {
        info!("Starting BlockAccessPlugin...");
        let context = self
            .context
            .as_ref()
            .expect("Plugin must be initialized before starting")
            .clone();

        let config = &context.config.plugins.block_access_service;
        if !config.enabled {
            info!("BlockAccessPlugin is disabled. Skipping start.");
            return Ok(());
        }

        let block_reader = {
            let providers = context.service_providers.read().unwrap();
            let key = TypeId::of::<BlockReaderProvider>();

            if let Some(provider_any) = providers.get(&key) {
                if let Some(provider_handle) = provider_any.downcast_ref::<BlockReaderProvider>() {
                    info!("Successfully retrieved BlockReaderProvider handle.");
                    provider_handle.get_service()
                } else {
                    return Err(anyhow::anyhow!("FATAL: Failed to downcast BlockReaderProvider.").into());
                }
            } else {
                warn!("BlockReaderProvider not found. The service will not be able to serve blocks.");
                return Ok(());
            }
        };

        let listen_address = format!("{}:{}", config.grpc_address, config.grpc_port);
        
        let service = BlockAccessServiceImpl { 
            block_reader,
            metrics: context.metrics.clone(),
        };
        let server = BlockAccessServiceServer::new(service);

        tokio::spawn(async move {
            info!("BlockAccess gRPC service listening on {}", listen_address);
            if let Err(e) = tonic::transport::Server::builder()
                .add_service(server)
                .serve(listen_address.parse().unwrap())
                .await
            {
                error!("gRPC server failed: {}", e);
            }
        });

        Ok(())
    }
}
