# Registry Isolation Guide

## Overview

This guide explains how to use Prometheus registry isolation in Rock Node tests to prevent cardinality conflicts during test execution.

## Problem

When multiple tests register Prometheus metrics with the same names, it causes cardinality conflicts:

```
thread 'test_name' panicked at prometheus-0.14.0/src/vec.rs:296:49:
called `Result::unwrap()` on an `Err` value: InconsistentCardinality { expect: 0, got: 1 }
```

This happens because Prometheus uses a global registry by default, and metrics with the same name cannot be registered multiple times.

## Solution

Rock Node implements registry isolation by:

1. **Providing isolated registries** for each test
2. **Using shared utilities** to ensure consistency
3. **Maintaining backward compatibility** with existing code

## Usage

### Basic Usage

```rust
use rock_node_core::test_utils::create_isolated_metrics;

// In your test
#[test]
fn my_test() {
    let metrics = create_isolated_metrics();
    // Use metrics...
}
```

### Plugin-Specific Helpers

For plugins that need `Arc<MetricsRegistry>`:

```rust
fn create_test_metrics() -> Arc<MetricsRegistry> {
    use rock_node_core::test_utils::create_isolated_metrics;
    Arc::new(create_isolated_metrics())
}
```

### AppContext with Isolated Metrics

For tests that need a full AppContext:

```rust
fn make_context_with_registry(metrics: MetricsRegistry) -> AppContext {
    // Create AppContext with injected metrics...
}

#[test]
fn my_plugin_test() {
    let metrics = create_isolated_metrics();
    let context = make_context_with_registry(metrics);
    // Test implementation...
}
```

## Implementation Details

### Shared Utilities

The `rock_node_core::test_utils` module provides:

- `create_isolated_metrics()` - Creates a fresh MetricsRegistry with isolated registry
- `create_isolated_metrics_with_prefix()` - Future enhancement for custom prefixes

### Registry Creation

Each call to `create_isolated_metrics()` creates:
1. A fresh `prometheus::Registry` instance
2. A new `MetricsRegistry` using `MetricsRegistry::with_registry()`
3. Isolated metric namespaces that don't conflict with other tests

### API Enhancement

The `MetricsRegistry` now supports:

```rust
// Production usage (unchanged)
let metrics = MetricsRegistry::new()?;

// Test usage (new)
let registry = prometheus::Registry::new();
let metrics = MetricsRegistry::with_registry(registry)?;
```

## Best Practices

### For Plugin Authors

1. **Always use isolated registries** in tests
2. **Avoid direct `prometheus::Registry` usage** in test code
3. **Use shared utilities** instead of duplicating isolation logic
4. **Document test requirements** clearly

### Example Plugin Structure

```rust
#[cfg(test)]
mod tests {
    use rock_node_core::test_utils::create_isolated_metrics;

    fn create_test_context() -> AppContext {
        let metrics = Arc::new(create_isolated_metrics());
        // ... create context with isolated metrics
    }

    #[test]
    fn test_plugin_functionality() {
        let context = create_test_context();
        // ... test implementation
    }
}
```

## Migration Guide

### Migrating from Direct Registry Creation

**Before:**
```rust
let registry = prometheus::Registry::new();
let metrics = Arc::new(MetricsRegistry::with_registry(registry).unwrap());
```

**After:**
```rust
let metrics = Arc::new(create_isolated_metrics());
```

### Migrating from MetricsRegistry::new()

**Before:**
```rust
let metrics = Arc::new(MetricsRegistry::new().unwrap());
```

**After:**
```rust
let metrics = Arc::new(create_isolated_metrics());
```

## Troubleshooting

### Common Issues

1. **Still getting cardinality errors?**
   - Ensure all test contexts use `create_isolated_metrics()`
   - Check for any direct `prometheus::Registry::new()` usage

2. **Metrics not appearing in tests?**
   - Verify the isolated registry is being used correctly
   - Check that metrics are registered with the correct registry

3. **Performance concerns?**
   - Isolated registries have minimal performance impact
   - Consider registry pooling for high-volume test scenarios

### Debug Tips

Enable debug logging to see registry operations:

```rust
#[test]
fn debug_test() {
    let _ = tracing_subscriber::fmt().with_max_level(tracing::Level::DEBUG).init();
    let metrics = create_isolated_metrics();
    // ... test with debug output
}
```

## Future Enhancements

- **Registry pooling** for better performance
- **Custom metric prefixes** for advanced isolation
- **Global test configuration** for isolation behavior
- **CI/CD integration** improvements

## Related Documentation

- [Metrics Implementation](../crates/rock-node-core/src/metrics.rs)
- [Test Utils Implementation](../crates/rock-node-core/src/test_utils.rs)
- [Contributing Guidelines](../CONTRIBUTING.md)

## Examples

See the implementation in:
- `crates/rock-node-persistence-plugin/src/cold_storage/archiver.rs`
- `crates/rock-node-publish-plugin/src/session_manager.rs`
- `crates/rock-node-core/src/test_utils.rs`
