use async_trait::async_trait;
use rock_node_core::{
    app_context::AppContext,
    error::{Error as CoreError, Result as CoreResult},
    plugin::Plugin,
    BlockReaderProvider,
};
use rock_node_protobufs::org::hiero::block::api::{
    block_stream_publish_service_server::BlockStreamPublishServiceServer,
    publish_stream_response::{self, end_of_stream},
    PublishStreamResponse,
};
use service::PublishServiceImpl;
use state::SharedState;
use std::{
    any::TypeId,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tonic::service::RoutesBuilder;
use tracing::{info, warn};

mod service;
mod session_manager;
mod state;

#[derive(Debug, Default)]
pub struct PublishPlugin {
    context: Option<AppContext>,
    running: Arc<AtomicBool>,
    pub shared_state: Option<Arc<SharedState>>,
}

impl PublishPlugin {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Plugin for PublishPlugin {
    fn name(&self) -> &'static str {
        "publish-plugin"
    }

    fn initialize(&mut self, context: AppContext) -> CoreResult<()> {
        info!("PublishPlugin initializing...");
        if !context.config.plugins.publish_service.enabled {
            info!("PublishPlugin is disabled.");
            return Ok(());
        }

        self.context = Some(context.clone());
        let shared_state = Arc::new(SharedState::new());
        self.shared_state = Some(shared_state.clone());

        let providers = context.service_providers.read().map_err(|e| {
            CoreError::PluginInitialization(format!("Failed to lock providers: {}", e))
        })?;

        if let Some(provider) = providers
            .get(&TypeId::of::<BlockReaderProvider>())
            .and_then(|p| p.downcast_ref::<BlockReaderProvider>())
        {
            let latest = provider
                .get_reader()
                .get_latest_persisted_block_number()?
                .unwrap_or(0);
            shared_state.set_latest_persisted_block(latest as i64);
        } else {
            warn!("BlockReaderProvider not found; latest persisted block state may be stale.");
            shared_state.set_latest_persisted_block(-1);
        }

        Ok(())
    }

    fn start(&mut self) -> CoreResult<()> {
        let context = self.context.as_ref().ok_or_else(|| {
            CoreError::PluginInitialization("PublishPlugin not initialized".to_string())
        })?;
        if !context.config.plugins.publish_service.enabled {
            return Ok(());
        }

        info!("Starting PublishPlugin background tasks...");
        let config = &context.config.plugins.publish_service;

        let cleanup_interval = Duration::from_secs(config.winner_cleanup_interval_seconds);
        let cleanup_threshold = config.winner_cleanup_threshold_blocks;
        let state_clone_for_cleanup = self.shared_state.as_ref().unwrap().clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(cleanup_interval).await;
                let latest_persisted = state_clone_for_cleanup.get_latest_persisted_block();
                if latest_persisted > 0 && (latest_persisted as u64) > cleanup_threshold {
                    let cleanup_before_block = (latest_persisted as u64) - cleanup_threshold;
                    state_clone_for_cleanup
                        .block_winners
                        .retain(|&k, _| k >= cleanup_before_block);
                }
            }
        });

        self.running.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn register_grpc_services(&mut self, builder: &mut RoutesBuilder) -> CoreResult<bool> {
        let context = self.context.as_ref().ok_or_else(|| {
            CoreError::PluginInitialization("PublishPlugin not initialized".to_string())
        })?;

        if !context.config.plugins.publish_service.enabled {
            return Ok(false);
        }

        let service = PublishServiceImpl {
            context: context.clone(),
            shared_state: self.shared_state.as_ref().unwrap().clone(),
        };
        const MAX_MESSAGE_SIZE: usize = 1024 * 1024 * 32;
        let server = BlockStreamPublishServiceServer::new(service)
            .max_decoding_message_size(MAX_MESSAGE_SIZE);

        builder.add_service(server);
        Ok(true)
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    async fn stop(&mut self) -> CoreResult<()> {
        if !self.is_running() {
            return Ok(());
        }
        info!("Stopping PublishPlugin...");

        if let Some(shared_state) = &self.shared_state {
            let latest_block = shared_state.get_latest_persisted_block();
            let end_stream_msg = PublishStreamResponse {
                response: Some(publish_stream_response::Response::EndStream(
                    publish_stream_response::EndOfStream {
                        status: end_of_stream::Code::Error as i32,
                        block_number: if latest_block < 0 {
                            0
                        } else {
                            latest_block as u64
                        },
                    },
                )),
            };
            for entry in shared_state.active_sessions.iter() {
                let _ = entry.value().send(Ok(end_stream_msg.clone())).await;
            }
        }

        self.running.store(false, Ordering::SeqCst);
        info!("PublishPlugin stopped.");
        Ok(())
    }
}
