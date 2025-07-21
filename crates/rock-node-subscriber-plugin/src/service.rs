use crate::session::SubscriberSession;
use dashmap::DashMap;
use rock_node_core::app_context::AppContext;
use rock_node_protobufs::org::hiero::block::api::{
    block_stream_subscribe_service_server::BlockStreamSubscribeService, SubscribeStreamRequest,
    SubscribeStreamResponse,
};
use std::sync::Arc;
use tokio::sync::{mpsc, Notify};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};
use tracing::{error, info};
use uuid::Uuid;

/// Implements the gRPC service for subscribing to block streams.
#[derive(Debug)]
pub struct SubscriberServiceImpl {
    context: Arc<AppContext>,
    // A signal that is cloned to each session to notify it of server shutdown.
    shutdown_notify: Arc<Notify>,
    // Keep track of active sessions to gracefully terminate them.
    active_sessions: Arc<DashMap<Uuid, Arc<Notify>>>,
}

impl SubscriberServiceImpl {
    pub fn new(context: Arc<AppContext>, shutdown_notify: Arc<Notify>) -> Self {
        Self {
            context,
            shutdown_notify,
            active_sessions: Arc::new(DashMap::new()),
        }
    }
}

#[tonic::async_trait]
impl BlockStreamSubscribeService for SubscriberServiceImpl {
    type subscribeBlockStreamStream = ReceiverStream<Result<SubscribeStreamResponse, Status>>;

    async fn subscribe_block_stream(
        &self,
        request: Request<SubscribeStreamRequest>,
    ) -> Result<Response<Self::subscribeBlockStreamStream>, Status> {
        let remote_addr = request.remote_addr();
        let request = request.into_inner();
        info!(
            ?request,
            ?remote_addr,
            "Received new subscribeBlockStream request."
        );

        let (tx, rx) = mpsc::channel(16);
        let session_shutdown_notify = Arc::new(Notify::new());

        let mut session = match SubscriberSession::new(
            self.context.clone(),
            request,
            tx,
            self.shutdown_notify.clone(),
            session_shutdown_notify.clone(),
        ) {
            Ok(session) => session,
            Err(status) => {
                error!("Failed to create subscriber session: {}", status);
                return Err(status);
            }
        };
        let session_id = session.id;
        self.active_sessions
            .insert(session_id, session_shutdown_notify);
        let active_sessions_clone = self.active_sessions.clone();

        tokio::spawn(async move {
            info!(session_id = %session.id, "Spawning new session handler task.");
            session.run().await;
            active_sessions_clone.remove(&session_id);
            info!(session_id = %session.id, "Session handler task finished.");
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }
}
