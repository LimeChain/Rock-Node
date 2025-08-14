mod worker;

use crate::worker::BackfillWorker;
use async_trait::async_trait;
use rock_node_core::{
    app_context::AppContext,
    config::BackfillMode,
    error::{Error as CoreError, Result as CoreResult},
    plugin::Plugin,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::Notify;
use tracing::{info, warn};

#[derive(Debug, Default)]
pub struct BackfillPlugin {
    context: Option<AppContext>,
    running: Arc<AtomicBool>,
    shutdown_notify: Arc<Notify>,
}

impl BackfillPlugin {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Plugin for BackfillPlugin {
    fn name(&self) -> &'static str {
        "backfill-plugin"
    }

    fn initialize(&mut self, context: AppContext) -> CoreResult<()> {
        self.context = Some(context);
        info!("BackfillPlugin initialized.");
        Ok(())
    }

    fn start(&mut self) -> CoreResult<()> {
        let context = self.context.as_ref().cloned().ok_or_else(|| {
            CoreError::PluginInitialization("BackfillPlugin not initialized".to_string())
        })?;
        let config = &context.config.plugins.backfill;

        if !config.enabled {
            info!("BackfillPlugin is disabled via configuration.");
            return Ok(());
        }

        if config.peers.is_empty() {
            warn!("BackfillPlugin is enabled but has no peers configured. Disabling plugin.");
            return Ok(());
        }

        let worker = match BackfillWorker::new(context.clone()) {
            Ok(s) => Arc::new(s),
            Err(e) => return Err(CoreError::PluginInitialization(e.to_string())),
        };

        let shutdown_notify = self.shutdown_notify.clone();
        self.running.store(true, Ordering::SeqCst);
        let running_clone = self.running.clone();
        let mode = config.mode.clone();

        tokio::spawn(async move {
            info!("Starting Backfill background task in {:?} mode.", mode);

            match mode {
                BackfillMode::GapFill => {
                    worker.run_gap_fill_loop(shutdown_notify).await;
                }
                BackfillMode::Continuous => {
                    worker.run_continuous_loop(shutdown_notify).await;
                }
            }

            running_clone.store(false, Ordering::SeqCst);
            info!("Backfill task has terminated.");
        });

        Ok(())
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    async fn stop(&mut self) -> CoreResult<()> {
        self.shutdown_notify.notify_waiters();
        self.running.store(false, Ordering::SeqCst);
        info!("BackfillPlugin stopped.");
        Ok(())
    }
}
