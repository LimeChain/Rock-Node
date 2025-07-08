use crate::service::SubscriberServiceImpl;
use rock_node_core::{app_context::AppContext, plugin::Plugin};
use rock_node_protobufs::org::hiero::block::api::block_stream_subscribe_service_server::BlockStreamSubscribeServiceServer;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tracing::info;

mod error;
mod service;
mod session;

pub struct SubscriberPlugin {
    context: Option<Arc<AppContext>>,
}

impl SubscriberPlugin {
    pub fn new() -> Self {
        Self { context: None }
    }
}

impl Default for SubscriberPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for SubscriberPlugin {
    fn name(&self) -> &'static str {
        "subscriber-plugin"
    }

    // Use the fully qualified, unambiguous Result type to match the trait definition
    fn initialize(&mut self, context: AppContext) -> rock_node_core::Result<()> {
        self.context = Some(Arc::new(context));
        info!("SubscriberPlugin initialized.");
        Ok(())
    }

    // Use the fully qualified, unambiguous Result type to match the trait definition
    fn start(&mut self) -> rock_node_core::Result<()> {
        info!("Starting SubscriberPlugin gRPC Server...");
        let context = self
            .context
            .as_ref()
            .expect("Plugin should be initialized before starting")
            .clone();

        let config = &context.config.plugins.subscriber_service;
        if !config.enabled {
            info!("SubscriberPlugin is disabled. Skipping start.");
            return Ok(());
        }
        let listen_address = format!("{}:{}", config.grpc_address, config.grpc_port);

        let service = SubscriberServiceImpl::new(context);
        let server = BlockStreamSubscribeServiceServer::new(service);

        tokio::spawn(async move {
            info!("Subscriber gRPC service listening on {}", listen_address);

            let listener = TcpListener::bind(&listen_address).await.unwrap();
            let listener_stream = TcpListenerStream::new(listener);

            if let Err(e) = tonic::transport::Server::builder()
                .add_service(server)
                .serve_with_incoming(listener_stream)
                .await
            {
                tracing::error!("Subscriber gRPC server failed: {}", e);
            }
        });

        Ok(())
    }
}
