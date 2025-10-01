use crate::fixtures::{mock_plugins::MockPlugin, test_context::IntegrationTestContext};
use rock_node_core::{capability::Capability, database_provider::DatabaseManagerProvider};
use rock_node_publish_plugin::PublishPlugin;
use std::any::TypeId;
use tracing_test::traced_test;

#[tokio::test]
#[traced_test]
async fn test_plugin_initialization_order() {
    // Create context with multiple plugins
    let mut ctx = IntegrationTestContext::builder()
        .with_plugin(Box::new(MockPlugin::new("plugin-1")))
        .with_plugin(Box::new(MockPlugin::new("plugin-2")))
        .with_plugin(Box::new(MockPlugin::new("plugin-3")))
        .build()
        .await
        .unwrap();

    // Initialize all plugins
    ctx.initialize_plugins().unwrap();

    // All plugins should be initialized
    assert_eq!(ctx.plugin_count(), 3);
}

#[tokio::test]
#[traced_test]
async fn test_persistence_requires_database() {
    // Create context with database provider
    let ctx = IntegrationTestContext::builder().build().await.unwrap();

    // Verify DatabaseManagerProvider is available
    let providers = ctx.context.service_providers.read().unwrap();
    assert!(providers
        .get(&TypeId::of::<DatabaseManagerProvider>())
        .is_some());
}

#[tokio::test]
#[traced_test]
async fn test_publish_plugin_initialization() {
    let mut ctx = IntegrationTestContext::builder()
        .with_plugin(Box::new(PublishPlugin::new()))
        .build()
        .await
        .unwrap();

    // Initialize plugin
    ctx.initialize_plugins().unwrap();

    // Plugin should not fail initialization (even without BlockReader, it should handle gracefully)
    assert_eq!(ctx.plugin_count(), 1);
}

#[tokio::test]
#[traced_test]
async fn test_capability_registration() {
    let mut ctx = IntegrationTestContext::builder()
        .with_plugin(Box::new(MockPlugin::new("test-plugin")))
        .build()
        .await
        .unwrap();

    ctx.initialize_plugins().unwrap();
    ctx.start_plugins().unwrap();

    // Capability registry should be accessible
    let cap_registry = &ctx.context.capability_registry;
    // Note: MockPlugin doesn't register capabilities, but we can test the mechanism exists
    assert!(
        !cap_registry
            .is_registered(Capability::ProvidesBlockReader)
            .await
    );
}

#[tokio::test]
#[traced_test]
async fn test_parallel_plugin_initialization() {
    // Create multiple plugins and initialize them
    let mut ctx = IntegrationTestContext::builder()
        .with_plugin(Box::new(MockPlugin::new("plugin-1")))
        .with_plugin(Box::new(MockPlugin::new("plugin-2")))
        .with_plugin(Box::new(MockPlugin::new("plugin-3")))
        .with_plugin(Box::new(MockPlugin::new("plugin-4")))
        .with_plugin(Box::new(MockPlugin::new("plugin-5")))
        .build()
        .await
        .unwrap();

    // Initialize should not deadlock
    let result = ctx.initialize_plugins();
    assert!(result.is_ok());

    // Start should work
    let result = ctx.start_plugins();
    assert!(result.is_ok());

    // All should be running
    assert!(ctx.all_plugins_running());

    // Stop should work
    ctx.stop_plugins().await.unwrap();
    assert!(!ctx.all_plugins_running());
}

#[tokio::test]
#[traced_test]
async fn test_plugin_start_requires_initialization() {
    let mut ctx = IntegrationTestContext::builder()
        .with_plugin(Box::new(MockPlugin::new("test-plugin")))
        .build()
        .await
        .unwrap();

    // Try to start without initializing
    let result = ctx.start_plugins();
    assert!(result.is_err());
}

#[tokio::test]
#[traced_test]
async fn test_plugin_lifecycle_complete() {
    let mut ctx = IntegrationTestContext::builder()
        .with_plugin(Box::new(MockPlugin::new("lifecycle-test")))
        .build()
        .await
        .unwrap();

    // Test complete lifecycle
    assert_eq!(ctx.stopped_plugin_names().len(), 1);

    ctx.initialize_plugins().unwrap();
    assert_eq!(ctx.stopped_plugin_names().len(), 1);

    ctx.start_plugins().unwrap();
    assert_eq!(ctx.running_plugin_names().len(), 1);
    assert_eq!(ctx.stopped_plugin_names().len(), 0);

    ctx.stop_plugins().await.unwrap();
    assert_eq!(ctx.stopped_plugin_names().len(), 1);
    assert_eq!(ctx.running_plugin_names().len(), 0);
}

#[tokio::test]
#[traced_test]
async fn test_plugin_initialization_failure_handling() {
    let mut ctx = IntegrationTestContext::builder()
        .with_plugin(Box::new(MockPlugin::new("good-plugin")))
        .with_plugin(Box::new(MockPlugin::new("bad-plugin").with_init_failure()))
        .build()
        .await
        .unwrap();

    // Initialization should fail
    let result = ctx.initialize_plugins();
    assert!(result.is_err());
}

#[tokio::test]
#[traced_test]
async fn test_plugin_start_failure_handling() {
    let mut ctx = IntegrationTestContext::builder()
        .with_plugin(Box::new(MockPlugin::new("good-plugin")))
        .with_plugin(Box::new(MockPlugin::new("bad-plugin").with_start_failure()))
        .build()
        .await
        .unwrap();

    ctx.initialize_plugins().unwrap();

    // Start should fail
    let result = ctx.start_plugins();
    assert!(result.is_err());
}

#[tokio::test]
#[traced_test]
async fn test_multiple_contexts_isolated() {
    // Create two independent contexts
    let mut ctx1 = IntegrationTestContext::builder()
        .with_plugin(Box::new(MockPlugin::new("ctx1-plugin")))
        .build()
        .await
        .unwrap();

    let mut ctx2 = IntegrationTestContext::builder()
        .with_plugin(Box::new(MockPlugin::new("ctx2-plugin")))
        .build()
        .await
        .unwrap();

    // Initialize both
    ctx1.initialize_plugins().unwrap();
    ctx2.initialize_plugins().unwrap();

    // They should have different temp directories
    assert_ne!(ctx1.temp_dir(), ctx2.temp_dir());

    // They should have different database paths
    assert_ne!(ctx1.database_path(), ctx2.database_path());

    // Cleanup
    ctx1.stop_plugins().await.unwrap();
    ctx2.stop_plugins().await.unwrap();
}

#[tokio::test]
#[traced_test]
async fn test_context_builder_default() {
    let ctx = IntegrationTestContext::builder().build().await.unwrap();

    // Should have sensible defaults
    assert_eq!(ctx.plugin_count(), 0);
    assert!(ctx.temp_dir().exists());
    assert_eq!(ctx.context.config.core.grpc_port, 0); // Random port for tests
}

#[tokio::test]
#[traced_test]
async fn test_context_with_config_overrides() {
    let ctx = IntegrationTestContext::builder()
        .with_config_override("core.log_level".to_string(), "DEBUG".to_string())
        .with_config_override("core.start_block_number".to_string(), "100".to_string())
        .build()
        .await
        .unwrap();

    assert_eq!(ctx.context.config.core.log_level, "DEBUG");
    assert_eq!(ctx.context.config.core.start_block_number, 100);
}
