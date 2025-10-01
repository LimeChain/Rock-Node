# Integration Tests

This crate contains integration tests for Rock Node plugin interactions. Unlike the `e2e` tests which run the full system in Docker, these tests focus on testing plugin-to-plugin interactions programmatically in a pure Rust environment.

## Test Structure

- **fixtures/** - Test utilities and builders
  - `test_context.rs` - Builder for creating isolated test contexts with plugins
  - `test_data.rs` - Generators for test blocks, events, and data
  - `mock_plugins.rs` - Lightweight plugin mocks for testing

- **tests/** - Test suites
  - `plugin_lifecycle.rs` - Plugin initialization order and dependencies
  - `shared_resources.rs` - Database, metrics, and cache sharing
  - `event_flow.rs` - Cross-plugin event propagation
  - `publish_persist_flow.rs` - End-to-end publish→verify→persist workflow

## Running Tests

```bash
# Run all integration tests
cargo test --package integration-tests

# Run specific test suite
cargo test --package integration-tests plugin_lifecycle

# Run with logging
RUST_LOG=debug cargo test --package integration-tests -- --nocapture
```

## Test Philosophy

These tests are designed to:
- ✅ Be **fast** (no Docker, minimal I/O)
- ✅ Be **deterministic** (no network, controlled state)
- ✅ Test **plugin interactions** (not full system behavior)
- ✅ Catch **integration bugs early** (before e2e tests)
- ✅ Be **easy to debug** (direct access to internal state)

## Adding New Tests

1. Use `IntegrationTestContext::builder()` to set up your test environment
2. Add only the plugins you need for your test
3. Use the provided fixtures for test data
4. Clean up resources properly (context does this automatically)

Example:
```rust
#[tokio::test]
async fn test_my_plugin_interaction() {
    let mut ctx = IntegrationTestContext::builder()
        .with_plugin(Box::new(PersistencePlugin::new(...)))
        .with_plugin(Box::new(PublishPlugin::new()))
        .build()
        .await
        .unwrap();

    ctx.initialize_plugins().unwrap();
    ctx.start_plugins().unwrap();

    // Your test logic here

    ctx.stop_plugins().await.unwrap();
}
```
