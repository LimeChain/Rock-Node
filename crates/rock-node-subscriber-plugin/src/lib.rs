use crate::service::SubscriberServiceImpl;
use async_trait::async_trait;
use rock_node_core::{
    app_context::AppContext,
    error::{Error as CoreError, Result as CoreResult},
    plugin::Plugin,
};
use rock_node_protobufs::org::hiero::block::api::block_stream_subscribe_service_server::BlockStreamSubscribeServiceServer;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::Notify;
use tonic::service::RoutesBuilder;
use tracing::info;

mod error;
mod service;
mod session;

#[derive(Debug, Default)]
pub struct SubscriberPlugin {
    context: Option<AppContext>,
    running: Arc<AtomicBool>,
    service_shutdown_notify: Arc<Notify>,
}

impl SubscriberPlugin {
    pub fn new() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            service_shutdown_notify: Arc::new(Notify::new()),
            context: None,
        }
    }
}

#[async_trait]
impl Plugin for SubscriberPlugin {
    fn name(&self) -> &'static str {
        "subscriber-plugin"
    }

    fn initialize(&mut self, context: AppContext) -> CoreResult<()> {
        info!("SubscriberPlugin initializing...");
        self.context = Some(context);
        Ok(())
    }

    fn start(&mut self) -> CoreResult<()> {
        let context = self.context.as_ref().ok_or_else(|| {
            CoreError::PluginInitialization("SubscriberPlugin not initialized".to_string())
        })?;

        if context.config.plugins.subscriber_service.enabled {
            self.running.store(true, Ordering::SeqCst);
        }
        Ok(())
    }

    fn register_grpc_services(&mut self, builder: &mut RoutesBuilder) -> CoreResult<bool> {
        let context = self.context.as_ref().ok_or_else(|| {
            CoreError::PluginInitialization("SubscriberPlugin not initialized".to_string())
        })?;

        if !context.config.plugins.subscriber_service.enabled {
            return Ok(false);
        }

        let service = SubscriberServiceImpl::new(
            Arc::new(context.clone()),
            self.service_shutdown_notify.clone(),
        );
        let server = BlockStreamSubscribeServiceServer::new(service);
        builder.add_service(server);
        Ok(true)
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    async fn stop(&mut self) -> CoreResult<()> {
        info!("Stopping SubscriberPlugin...");
        self.service_shutdown_notify.notify_waiters();
        self.running.store(false, Ordering::SeqCst);
        Ok(())
    }
}
