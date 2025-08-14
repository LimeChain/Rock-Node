use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Config {
    #[serde(default)]
    pub core: CoreConfig,
    #[serde(default)]
    pub plugins: PluginConfigs,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct CoreConfig {
    pub log_level: String,
    pub database_path: String,
    #[serde(default)]
    pub start_block_number: u64,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            log_level: "INFO".to_string(),
            database_path: "data/database".to_string(),
            start_block_number: 0,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
#[serde(default)]
pub struct PluginConfigs {
    pub observability: ObservabilityConfig,
    pub persistence_service: PersistenceServiceConfig,
    pub publish_service: PublishServiceConfig,
    pub verification_service: VerificationServiceConfig,
    pub block_access_service: BlockAccessServiceConfig,
    pub server_status_service: ServerStatusServiceConfig,
    pub state_management_service: StateManagementServiceConfig,
    pub subscriber_service: SubscriberServiceConfig,
    pub query_service: QueryServiceConfig,
    #[serde(default)]
    pub backfill: BackfillConfig,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct ObservabilityConfig {
    pub enabled: bool,
    pub listen_address: String,
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            listen_address: "0.0.0.0:9600".to_string(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct PersistenceServiceConfig {
    pub enabled: bool,
    pub cold_storage_path: String,
    pub hot_storage_block_count: u64,
    pub archive_batch_size: u64,
}

impl Default for PersistenceServiceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            cold_storage_path: "data/cold_storage".to_string(),
            hot_storage_block_count: 100,
            archive_batch_size: 100,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct StateManagementServiceConfig {
    pub enabled: bool,
}

impl Default for StateManagementServiceConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct PublishServiceConfig {
    pub enabled: bool,
    pub grpc_address: String,
    pub grpc_port: u16,
    pub max_concurrent_streams: usize,
    pub persistence_ack_timeout_seconds: u64,
    pub stale_winner_timeout_seconds: u64,
    pub winner_cleanup_interval_seconds: u64,
    pub winner_cleanup_threshold_blocks: u64,
}

impl Default for PublishServiceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            grpc_address: "0.0.0.0".to_string(),
            grpc_port: 8090,
            max_concurrent_streams: 250,
            persistence_ack_timeout_seconds: 60,
            stale_winner_timeout_seconds: 10,
            winner_cleanup_interval_seconds: 150,
            winner_cleanup_threshold_blocks: 1000,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct SubscriberServiceConfig {
    pub enabled: bool,
    pub grpc_address: String,
    pub grpc_port: u16,
    pub max_concurrent_streams: usize,
    pub session_timeout_seconds: u64,
    pub live_stream_queue_size: usize,
    pub max_future_block_lookahead: u64,
}

impl Default for SubscriberServiceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            grpc_address: "0.0.0.0".to_string(),
            grpc_port: 6895,
            max_concurrent_streams: 100,
            session_timeout_seconds: 60,
            live_stream_queue_size: 250,
            max_future_block_lookahead: 250,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct VerificationServiceConfig {
    pub enabled: bool,
}

impl Default for VerificationServiceConfig {
    fn default() -> Self {
        Self { enabled: false }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct BlockAccessServiceConfig {
    pub enabled: bool,
    pub grpc_address: String,
    pub grpc_port: u16,
}

impl Default for BlockAccessServiceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            grpc_address: "0.0.0.0".to_string(),
            grpc_port: 6791,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct ServerStatusServiceConfig {
    pub enabled: bool,
    pub grpc_address: String,
    pub grpc_port: u16,
}

impl Default for ServerStatusServiceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            grpc_address: "0.0.0.0".to_string(),
            grpc_port: 6792,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct QueryServiceConfig {
    pub enabled: bool,
    pub grpc_address: String,
    pub grpc_port: u16,
}

impl Default for QueryServiceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            grpc_address: "0.0.0.0".to_string(),
            grpc_port: 6793,
        }
    }
}


#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "kebab-case")]
pub enum BackfillMode {
    GapFill,
    Continuous,
}

impl Default for BackfillMode {
    fn default() -> Self {
        BackfillMode::GapFill
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(default)]
pub struct BackfillConfig {
    pub enabled: bool,
    #[serde(default)]
    pub mode: BackfillMode,
    pub peers: Vec<String>,
    pub check_interval_seconds: u64,
    #[serde(default = "default_max_batch_size")]
    pub max_batch_size: u64,
}

fn default_max_batch_size() -> u64 {
    1000
}

impl Default for BackfillConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: BackfillMode::GapFill,
            peers: vec![],
            check_interval_seconds: 60,
            max_batch_size: 1000,
        }
    }
}