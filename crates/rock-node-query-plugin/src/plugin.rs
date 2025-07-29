use crate::services::{
    consensus_service::ConsensusServiceImpl, contract_service::SmartContractServiceImpl,
    crypto_service::CryptoServiceImpl, file_service::FileServiceImpl,
    network_service::NetworkServiceImpl, schedule_service::ScheduleServiceImpl,
    token_service::TokenServiceImpl,
};
use async_trait::async_trait;
use rock_node_core::{
    app_context::AppContext,
    error::{Error as CoreError, Result as CoreResult},
    plugin::Plugin,
    state_reader::StateReaderProvider,
};
use rock_node_protobufs::proto::{
    consensus_service_server::ConsensusServiceServer, crypto_service_server::CryptoServiceServer,
    file_service_server::FileServiceServer, network_service_server::NetworkServiceServer,
    schedule_service_server::ScheduleServiceServer,
    smart_contract_service_server::SmartContractServiceServer,
    token_service_server::TokenServiceServer,
};
use std::{
    any::TypeId,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::sync::watch;
use tracing::{error, info, warn};

/// The main plugin struct that registers and runs the gRPC query services.
#[derive(Debug, Default)]
pub struct QueryPlugin {
    context: Option<AppContext>,
    running: Arc<AtomicBool>,
    shutdown_tx: Option<watch::Sender<()>>,
}

impl QueryPlugin {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Plugin for QueryPlugin {
    fn name(&self) -> &'static str {
        "rock-node-query-plugin"
    }

    fn initialize(&mut self, context: AppContext) -> CoreResult<()> {
        info!("QueryPlugin initialized.");
        self.context = Some(context);
        self.running = Arc::new(AtomicBool::new(false));
        Ok(())
    }

    fn start(&mut self) -> CoreResult<()> {
        let context = self
            .context
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("QueryPlugin not initialized"))?
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
        let socket_addr = listen_address.parse().map_err(|e| {
            anyhow::anyhow!(
                "Failed to parse gRPC listen address '{}': {}",
                listen_address,
                e
            )
        })?;

        let crypto_service = CryptoServiceImpl::new(state_reader.clone());
        let file_service = FileServiceImpl::new(state_reader.clone());
        let consensus_service = ConsensusServiceImpl::new(state_reader.clone());
        let network_service = NetworkServiceImpl::new(state_reader.clone());
        let schedule_service = ScheduleServiceImpl::new(state_reader.clone());
        let token_service = TokenServiceImpl::new(state_reader.clone());
        let smart_contract_service = SmartContractServiceImpl::new(state_reader.clone());

        let crypto_service_server = CryptoServiceServer::new(crypto_service);
        let file_service_server = FileServiceServer::new(file_service);
        let consensus_service_server = ConsensusServiceServer::new(consensus_service);
        let network_service_server = NetworkServiceServer::new(network_service);
        let schedule_service_server = ScheduleServiceServer::new(schedule_service);
        let token_service_server = TokenServiceServer::new(token_service);
        let smart_contract_service_server = SmartContractServiceServer::new(smart_contract_service);

        let (shutdown_tx, mut shutdown_rx) = watch::channel(());
        self.shutdown_tx = Some(shutdown_tx);
        let running_clone = self.running.clone();

        self.running.store(true, Ordering::SeqCst);
        tokio::spawn(async move {
            info!(
                "QueryPlugin: CryptoService gRPC listening on {}",
                socket_addr
            );

            let server_future = tonic::transport::Server::builder()
                .add_service(crypto_service_server)
                .add_service(file_service_server)
                .add_service(consensus_service_server)
                .add_service(network_service_server)
                .add_service(schedule_service_server)
                .add_service(token_service_server)
                .add_service(smart_contract_service_server)
                .serve_with_shutdown(socket_addr, async move {
                    shutdown_rx.changed().await.ok();
                    info!("Gracefully shutting down Query gRPC server...");
                });

            if let Err(e) = server_future.await {
                error!("QueryPlugin gRPC server failed: {}", e);
            }
            running_clone.store(false, Ordering::SeqCst);
        });

        Ok(())
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    async fn stop(&mut self) -> CoreResult<()> {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            if shutdown_tx.send(()).is_err() {
                let msg = "Failed to send shutdown signal to Query gRPC server: receiver dropped.";
                error!("{}", msg);
                return Err(CoreError::PluginShutdown(msg.to_string()));
            }
        }
        self.running.store(false, Ordering::SeqCst);
        Ok(())
    }
}
