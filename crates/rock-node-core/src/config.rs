use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct Config {
    pub core: CoreConfig,
    pub plugins: PluginConfigs,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CoreConfig {
    pub log_level: String,
    pub database_path: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PluginConfigs {
    pub observability: ObservabilityConfig,
    pub persistence: PersistenceConfig,
    pub subscribe_service: SubscribeServiceConfig,
    pub publish_service: PublishServiceConfig,
    // Add other plugin configs here
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ObservabilityConfig {
    pub enabled: bool,
    pub listen_address: String, 
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PublishServiceConfig {
    pub enabled: bool,
    pub grpc_address: String,
    pub grpc_port: u16,
    pub max_concurrent_streams: usize,
} 

#[derive(Debug, Deserialize, Serialize)]
pub struct PersistenceConfig {
    pub enabled: bool,
    pub hot_storage_path: String,
    pub cold_storage_path: String,
    pub archive_trigger_blocks: u64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SubscribeServiceConfig {
    pub enabled: bool,
    pub grpc_port: u16,
    pub max_concurrent_streams: usize,
} 