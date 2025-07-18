use crate::service::CryptoServiceImpl;
use rock_node_core::{state_reader::StateReaderProvider, AppContext, Plugin};
use rock_node_protobufs::proto::crypto_service_server::CryptoServiceServer;
use std::any::TypeId;
use tracing::{error, info, warn};

/// The main plugin struct that registers and runs the gRPC query services.
#[derive(Debug, Default)]
pub struct QueryPlugin {
    context: Option<AppContext>,
}

impl QueryPlugin {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Plugin for QueryPlugin {
    fn name(&self) -> &'static str {
        "rock-node-query-plugin"
    }

    fn initialize(&mut self, context: AppContext) -> rock_node_core::Result<()> {
        info!("QueryPlugin initialized.");
        self.context = Some(context);
        Ok(())
    }

    fn start(&mut self) -> rock_node_core::Result<()> {
        let context = self
            .context
            .as_ref()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "QueryPlugin not initialized - initialize() must be called before start()"
                )
            })?
            .clone();
        let config = &context.config.plugins.query_service;

        if !config.enabled {
            info!("QueryPlugin is disabled. Skipping start.");
            return Ok(());
        }

        let state_reader = {
            let providers = context
                .service_providers
                .read()
                .map_err(|_| anyhow::anyhow!("Failed to acquire read lock on service providers"))?;
            providers
                .get(&TypeId::of::<StateReaderProvider>())
                .and_then(|p| p.downcast_ref::<StateReaderProvider>())
                .map(|p| p.get_reader())
                .ok_or_else(|| {
                    warn!("StateReaderProvider not found. QueryPlugin cannot start.");
                    rock_node_core::Error::PluginInitialization(
                        "StateReaderProvider not found".to_string(),
                    )
                })?
        };

        let listen_address = format!("{}:{}", config.grpc_address, config.grpc_port);
        let crypto_service = CryptoServiceImpl::new(state_reader);
        let server = CryptoServiceServer::new(crypto_service);

        // Parse the address before spawning to handle errors properly
        let socket_addr = listen_address.parse().map_err(|e| {
            anyhow::anyhow!(
                "Failed to parse gRPC listen address '{}': {}",
                listen_address,
                e
            )
        })?;

        info!(
            "QueryPlugin: CryptoService gRPC listening on {}",
            socket_addr
        );

        tokio::spawn(async move {
            if let Err(e) = tonic::transport::Server::builder()
                .add_service(server)
                .serve(socket_addr)
                .await
            {
                error!("QueryPlugin gRPC server failed: {}", e);
            }
        });

        Ok(())
    }
}
