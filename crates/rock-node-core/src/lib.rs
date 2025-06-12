// File: crates/rock-node-core/src/lib.rs

// Declare all the modules in our crate
pub mod app_context;
pub mod capability;
pub mod config;
pub mod error;
pub mod plugin;

// Re-export the most important public types for easy access by other crates.
pub use app_context::AppContext;
pub use capability::{Capability, CapabilityRegistry};
pub use config::Config;
pub use error::{Error, Result};
pub use plugin::Plugin;