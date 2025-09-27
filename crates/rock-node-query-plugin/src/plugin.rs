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
use tonic::service::RoutesBuilder;
use tracing::{info, warn};

/// The main plugin struct that registers and runs the gRPC query services.
#[derive(Debug, Default)]
pub struct QueryPlugin {
    context: Option<AppContext>,
    running: Arc<AtomicBool>,
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
        let context = self.context.as_ref().ok_or_else(|| {
            CoreError::PluginInitialization("QueryPlugin not initialized".to_string())
        })?;

        if context.config.plugins.query_service.enabled {
            self.running.store(true, Ordering::SeqCst);
        }
        Ok(())
    }

    fn register_grpc_services(&mut self, builder: &mut RoutesBuilder) -> CoreResult<bool> {
        let context = self.context.as_ref().ok_or_else(|| {
            CoreError::PluginInitialization("QueryPlugin not initialized".to_string())
        })?;

        if !context.config.plugins.query_service.enabled {
            return Ok(false);
        }

        let state_reader = {
            let providers = context.service_providers.read().map_err(|_| {
                CoreError::PluginInitialization(
                    "Failed to acquire read lock on service providers".to_string(),
                )
            })?;
            match providers
                .get(&TypeId::of::<StateReaderProvider>())
                .and_then(|p| p.downcast_ref::<StateReaderProvider>())
                .map(|p| p.get_reader())
            {
                Some(reader) => reader,
                None => {
                    warn!("StateReaderProvider not found. QueryPlugin cannot register services.");
                    return Ok(false); // Can't proceed without the state reader.
                },
            }
        };

        let crypto_service = CryptoServiceImpl::new(state_reader.clone());
        let file_service = FileServiceImpl::new(state_reader.clone());
        let consensus_service = ConsensusServiceImpl::new(state_reader.clone());
        let network_service = NetworkServiceImpl::new(state_reader.clone());
        let schedule_service = ScheduleServiceImpl::new(state_reader.clone());
        let token_service = TokenServiceImpl::new(state_reader.clone());
        let smart_contract_service = SmartContractServiceImpl::new(state_reader);

        builder
            .add_service(CryptoServiceServer::new(crypto_service))
            .add_service(FileServiceServer::new(file_service))
            .add_service(ConsensusServiceServer::new(consensus_service))
            .add_service(NetworkServiceServer::new(network_service))
            .add_service(ScheduleServiceServer::new(schedule_service))
            .add_service(TokenServiceServer::new(token_service))
            .add_service(SmartContractServiceServer::new(smart_contract_service));

        Ok(true)
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    async fn stop(&mut self) -> CoreResult<()> {
        self.running.store(false, Ordering::SeqCst);
        Ok(())
    }
}
