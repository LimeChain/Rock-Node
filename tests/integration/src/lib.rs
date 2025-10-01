//! Integration tests for Rock Node plugin interactions.
//!
//! This crate tests plugin-to-plugin interactions without requiring Docker or full system deployment.
//! Tests focus on:
//! - Plugin lifecycle and initialization order
//! - Shared resource management (database, metrics, cache)
//! - Cross-plugin event propagation
//! - End-to-end workflows through the plugin pipeline

pub mod fixtures;
pub mod tests;

// Re-export commonly used types for convenience
pub use fixtures::{test_context::IntegrationTestContext, test_data::TestDataBuilder};
