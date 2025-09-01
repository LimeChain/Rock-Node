use crate::service::SubscriberServiceImpl;
use async_trait::async_trait;
use rock_node_core::{app_context::AppContext, error::Result as CoreResult, plugin::Plugin};
use rock_node_protobufs::org::hiero::block::api::block_stream_subscribe_service_server::BlockStreamSubscribeServiceServer;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::Notify;
use tonic::transport::server::Router;
use tracing::info;

mod error;
mod service;
mod session;

pub struct SubscriberPlugin {
    running: Arc<AtomicBool>,
    service_shutdown_notify: Arc<Notify>,
    router: Option<Router>,
}

impl SubscriberPlugin {
    pub fn new() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            service_shutdown_notify: Arc::new(Notify::new()),
            router: None,
        }
    }
}

impl Default for SubscriberPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Plugin for SubscriberPlugin {
    fn name(&self) -> &'static str {
        "subscriber-plugin"
    }

    fn initialize(&mut self, context: AppContext) -> CoreResult<()> {
        info!("SubscriberPlugin initializing...");
        if !context.config.plugins.subscriber_service.enabled {
            info!("SubscriberPlugin is disabled.");
            return Ok(());
        }

        let service =
            SubscriberServiceImpl::new(Arc::new(context), self.service_shutdown_notify.clone());
        let server = BlockStreamSubscribeServiceServer::new(service);
        self.router = Some(tonic::transport::Server::builder().add_service(server));
        Ok(())
    }

    fn start(&mut self) -> CoreResult<()> {
        self.running.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn take_grpc_router(&mut self) -> Option<Router> {
        self.router.take()
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
