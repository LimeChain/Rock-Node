use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Config {
    pub core: CoreConfig,
    pub plugins: PluginConfigs,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct CoreConfig {
    pub log_level: String,
    pub database_path: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PluginConfigs {
    pub observability: ObservabilityConfig,
    pub persistence_service: PersistenceServiceConfig,
    pub subscribe_service: SubscribeServiceConfig,
    pub publish_service: PublishServiceConfig,
    pub verification_service: VerificationServiceConfig,
    pub block_access_service: BlockAccessServiceConfig,
    pub server_status_service: ServerStatusServiceConfig,
    pub state_service: StateServiceConfig,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ObservabilityConfig {
    pub enabled: bool,
    pub listen_address: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PersistenceServiceConfig {
    pub enabled: bool,
    pub cold_storage_path: String,
    pub hot_storage_block_count: u64,
    pub archive_batch_size: u64,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct StateServiceConfig {
    pub enabled: bool,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PublishServiceConfig {
    pub enabled: bool,
    pub grpc_address: String,
    pub grpc_port: u16,
    pub max_concurrent_streams: usize,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SubscribeServiceConfig {
    pub enabled: bool,
    pub grpc_port: u16,
    pub max_concurrent_streams: usize,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct VerificationServiceConfig {
    pub enabled: bool,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct BlockAccessServiceConfig {
    pub enabled: bool,
    pub grpc_address: String,
    pub grpc_port: u16,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ServerStatusServiceConfig {
    pub enabled: bool,
    pub grpc_address: String,
    pub grpc_port: u16,
}
