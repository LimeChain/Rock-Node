use crate::database::DatabaseManager;
use std::sync::Arc;

/// A simple provider struct to act as a handle for the shared DatabaseManager.
/// This helps ensure type safety when storing and retrieving from the AppContext.
#[derive(Clone)]
pub struct DatabaseManagerProvider {
    manager: Arc<DatabaseManager>,
}

impl DatabaseManagerProvider {
    pub fn new(manager: Arc<DatabaseManager>) -> Self {
        Self { manager }
    }

    pub fn get_manager(&self) -> Arc<DatabaseManager> {
        self.manager.clone()
    }
}
