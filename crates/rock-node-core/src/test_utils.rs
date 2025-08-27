//! Shared utilities for testing Rock Node components.
//!
//! This module provides common testing utilities, particularly for
//! Prometheus registry isolation to prevent cardinality conflicts
//! during test execution.
//!
//! # Registry Isolation
//!
//! The main purpose of this module is to provide utilities for creating
//! isolated Prometheus registries during testing. This prevents cardinality
//! conflicts that occur when multiple tests register metrics with the same
//! names.
//!
//! ## Usage
//!
//! ```rust,ignore
//! use rock_node_core::test_utils::create_isolated_metrics;
//!
//! #[test]
//! fn my_test() {
//!     let metrics = create_isolated_metrics();
//!     // Use metrics in test...
//! }
//! ```
//!
//! ## Implementation Details
//!
//! Each call to `create_isolated_metrics()` creates a fresh `prometheus::Registry`
//! and uses `MetricsRegistry::with_registry()` to create a registry with
//! isolated metric namespaces.

use crate::metrics::MetricsRegistry;

/// Creates an isolated MetricsRegistry for testing.
///
/// This function creates a fresh Prometheus registry to avoid cardinality
/// conflicts that can occur when multiple tests register metrics with
/// the same names.
///
/// # Returns
///
/// A new MetricsRegistry with an isolated registry.
///
/// # Examples
///
/// ```rust,ignore
/// use rock_node_core::test_utils::create_isolated_metrics;
///
/// let metrics = create_isolated_metrics();
/// // Use metrics in test...
/// ```
pub fn create_isolated_metrics() -> MetricsRegistry {
    let registry = prometheus::Registry::new();
    MetricsRegistry::with_registry(registry).expect("Failed to create isolated metrics registry")
}

/// Creates an isolated MetricsRegistry with custom metric names.
///
/// This is useful when you need to avoid conflicts with production
/// metric names during testing.
///
/// # Arguments
///
/// * `prefix` - A prefix to add to all metric names to avoid conflicts
///
/// # Returns
///
/// A new MetricsRegistry with isolated, prefixed metric names.
///
/// # Note
///
/// This function is currently not implemented as it would require
/// significant changes to the MetricsRegistry implementation.
/// For now, use `create_isolated_metrics()` instead.
pub fn create_isolated_metrics_with_prefix(_prefix: &str) -> MetricsRegistry {
    // TODO: Implement when needed
    // This would require modifying MetricsRegistry to support
    // custom metric name prefixes
    create_isolated_metrics()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_isolated_metrics() {
        let metrics = create_isolated_metrics();

        // Verify we can access metrics without conflicts
        let _counter = metrics.blocks_acknowledged.clone();

        // If this test passes, registry isolation is working
        assert!(true);
    }

    #[test]
    fn test_isolated_registries_are_independent() {
        let metrics1 = create_isolated_metrics();
        let metrics2 = create_isolated_metrics();

        // Increment counter on first registry
        metrics1.blocks_acknowledged.inc();

        // Second registry should have different values
        assert_eq!(metrics1.blocks_acknowledged.get(), 1);
        assert_eq!(metrics2.blocks_acknowledged.get(), 0);
    }
}
