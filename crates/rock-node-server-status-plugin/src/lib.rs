mod service;

use rock_node_core::{app_context::AppContext, error::Result, plugin::Plugin, BlockReaderProvider};
use rock_node_protobufs::org::hiero::block::api::block_node_service_server::BlockNodeServiceServer;
use service::StatusServiceImpl;
use std::any::TypeId;
use tracing::{error, info, warn};

#[derive(Debug, Default)]
pub struct StatusPlugin {
    context: Option<AppContext>,
}

impl StatusPlugin {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Plugin for StatusPlugin {
    fn name(&self) -> &'static str {
        "status-plugin"
    }

    fn initialize(&mut self, context: AppContext) -> Result<()> {
        info!("StatusPlugin initialized.");
        self.context = Some(context);
        Ok(())
    }

    fn start(&mut self) -> Result<()> {
        info!("Starting Server Status Plugin...");
        let context = self
            .context
            .as_ref()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "StatusPlugin not initialized - initialize() must be called before start()"
                )
            })?
            .clone();

        // Ensure the plugin is enabled in config before starting the server
        let config = &context.config.plugins.server_status_service;
        if !config.enabled {
            info!("Server StatusPlugin is disabled. Skipping start.");
            return Ok(());
        }

        // Get the BlockReader service provided by the persistence plugin
        let block_reader = {
            let providers = context
                .service_providers
                .read()
                .map_err(|_| anyhow::anyhow!("Failed to acquire read lock on service providers"))?;
            let key = TypeId::of::<BlockReaderProvider>();

            if let Some(provider_any) = providers.get(&key) {
                if let Some(provider_handle) = provider_any.downcast_ref::<BlockReaderProvider>() {
                    info!("Successfully retrieved BlockReaderProvider handle.");
                    provider_handle.get_service()
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

        // Pass the metrics registry to the service implementation
        let service = StatusServiceImpl {
            block_reader,
            metrics: context.metrics.clone(),
        };
        let server = BlockNodeServiceServer::new(service);

        // Parse the address before spawning to handle errors properly
        let socket_addr = listen_address.parse().map_err(|e| {
            anyhow::anyhow!(
                "Failed to parse gRPC listen address '{}': {}",
                listen_address,
                e
            )
        })?;

        tokio::spawn(async move {
            info!("Status gRPC service listening on {}", socket_addr);
            if let Err(e) = tonic::transport::Server::builder()
                .add_service(server)
                .serve(socket_addr)
                .await
            {
                error!("Status gRPC server failed: {}", e);
            }
        });

        Ok(())
    }
}
