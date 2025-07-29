# Rock-Node Configuration Reference

This document outlines how configuration is loaded and how every value found in `config/config.toml` can be overridden at runtime using environment variables.

The application loads configuration in **two layers (highest precedence last)**:

1. The TOML file (`config/config.toml` by default or one supplied via `--config-path`).
2. Environment variables that start with the prefix `ROCK_NODE__` (note the double underscores). Any matching variable **overrides** the corresponding value from the TOML file.

---

## Core

| TOML key           | Environment variable                 | Default value   | Description                                     |
|--------------------|---------------------------------------|-----------------|-------------------------------------------------|
| `core.log_level`   | `ROCK_NODE__CORE__LOG_LEVEL`          | `INFO`          | Global log level for `tracing` (`TRACE`, `INFO` …) |
| `core.database_path` | `ROCK_NODE__CORE__DATABASE_PATH`    | `data/database` | Directory that stores RocksDB data              |

---

## Plugins

### Observability

| TOML key                               | Environment variable                                                  | Default value  | Description |
|----------------------------------------|------------------------------------------------------------------------|---------------|-------------|
| `plugins.observability.enabled`        | `ROCK_NODE__PLUGINS__OBSERVABILITY__ENABLED`                           | `true`        | Enable metrics & Grafana dashboards |
| `plugins.observability.listen_address` | `ROCK_NODE__PLUGINS__OBSERVABILITY__LISTEN_ADDRESS`                    | `0.0.0.0:9600`| HTTP bind address for metrics |

### Publish Service

| TOML key                                                    | Environment variable                                                                        | Default | Description |
|-------------------------------------------------------------|---------------------------------------------------------------------------------------------|---------|-------------|
| `plugins.publish_service.enabled`                           | `ROCK_NODE__PLUGINS__PUBLISH_SERVICE__ENABLED`                                              | `true`  | Enable gRPC publish service |
| `plugins.publish_service.grpc_address`                      | `ROCK_NODE__PLUGINS__PUBLISH_SERVICE__GRPC_ADDRESS`                                         | `0.0.0.0` | Bind address |
| `plugins.publish_service.grpc_port`                         | `ROCK_NODE__PLUGINS__PUBLISH_SERVICE__GRPC_PORT`                                            | `8090`  | Port |
| `plugins.publish_service.max_concurrent_streams`            | `ROCK_NODE__PLUGINS__PUBLISH_SERVICE__MAX_CONCURRENT_STREAMS`                               | `250`   | Max simultaneous gRPC streams |
| `plugins.publish_service.persistence_ack_timeout_seconds`   | `ROCK_NODE__PLUGINS__PUBLISH_SERVICE__PERSISTENCE_ACK_TIMEOUT_SECONDS`                      | `60`    | Seconds to wait for block persistence acknowledgement |
| `plugins.publish_service.stale_winner_timeout_seconds`      | `ROCK_NODE__PLUGINS__PUBLISH_SERVICE__STALE_WINNER_TIMEOUT_SECONDS`                         | `60`    | Timeout before re-electing stale winners |
| `plugins.publish_service.winner_cleanup_interval_seconds`   | `ROCK_NODE__PLUGINS__PUBLISH_SERVICE__WINNER_CLEANUP_INTERVAL_SECONDS`                      | `300`   | Interval between cleanup passes |
| `plugins.publish_service.winner_cleanup_threshold_blocks`   | `ROCK_NODE__PLUGINS__PUBLISH_SERVICE__WINNER_CLEANUP_THRESHOLD_BLOCKS`                      | `1000`  | Blocks kept before a winner entry is purged |

### Persistence Service

| TOML key                                                     | Environment variable                                                                         | Default | Description |
|--------------------------------------------------------------|------------------------------------------------------------------------------------------------|---------|-------------|
| `plugins.persistence_service.enabled`                        | `ROCK_NODE__PLUGINS__PERSISTENCE_SERVICE__ENABLED`                                            | `true`  | Enable tiered persistence plugin |
| `plugins.persistence_service.cold_storage_path`              | `ROCK_NODE__PLUGINS__PERSISTENCE_SERVICE__COLD_STORAGE_PATH`                                  | `data/cold_storage` | Directory for archived blocks |
| `plugins.persistence_service.hot_storage_block_count`        | `ROCK_NODE__PLUGINS__PERSISTENCE_SERVICE__HOT_STORAGE_BLOCK_COUNT`                            | `100`   | Blocks retained in hot RocksDB tier |
| `plugins.persistence_service.archive_batch_size`             | `ROCK_NODE__PLUGINS__PERSISTENCE_SERVICE__ARCHIVE_BATCH_SIZE`                                 | `100`   | Blocks moved to cold tier in one batch |

### State Management Service

| TOML key                                         | Environment variable                                                        | Default | Description |
|--------------------------------------------------|------------------------------------------------------------------------------|---------|-------------|
| `plugins.state_management_service.enabled`       | `ROCK_NODE__PLUGINS__STATE_MANAGEMENT_SERVICE__ENABLED`                      | `true`  | Enable state-management plugin |

### Subscriber Service

| TOML key                                                        | Environment variable                                                                 | Default | Description |
|-----------------------------------------------------------------|--------------------------------------------------------------------------------------|---------|-------------|
| `plugins.subscriber_service.enabled`                             | `ROCK_NODE__PLUGINS__SUBSCRIBER_SERVICE__ENABLED`                                     | `true`  | Enable gRPC subscriber service |
| `plugins.subscriber_service.grpc_address`                        | `ROCK_NODE__PLUGINS__SUBSCRIBER_SERVICE__GRPC_ADDRESS`                                | `0.0.0.0` | Bind address |
| `plugins.subscriber_service.grpc_port`                           | `ROCK_NODE__PLUGINS__SUBSCRIBER_SERVICE__GRPC_PORT`                                   | `6895`  | Port |
| `plugins.subscriber_service.max_concurrent_streams`              | `ROCK_NODE__PLUGINS__SUBSCRIBER_SERVICE__MAX_CONCURRENT_STREAMS`                      | `100`   | Max simultaneous gRPC streams |
| `plugins.subscriber_service.live_stream_queue_size`              | `ROCK_NODE__PLUGINS__SUBSCRIBER_SERVICE__LIVE_STREAM_QUEUE_SIZE`                      | `250`   | Buffer for live block streams |
| `plugins.subscriber_service.max_future_block_lookahead`         | `ROCK_NODE__PLUGINS__SUBSCRIBER_SERVICE__MAX_FUTURE_BLOCK_LOOKAHEAD`                  | `250`   | Max future blocks a client may request |
| `plugins.subscriber_service.session_timeout_seconds`             | `ROCK_NODE__PLUGINS__SUBSCRIBER_SERVICE__SESSION_TIMEOUT_SECONDS`                     | `60`    | Inactive session timeout |

### Verification Service

| TOML key                                   | Environment variable                                          | Default | Description |
|--------------------------------------------|---------------------------------------------------------------|---------|-------------|
| `plugins.verification_service.enabled`     | `ROCK_NODE__PLUGINS__VERIFICATION_SERVICE__ENABLED`           | `false` | Enable verification plugin |

### Block-Access Service

| TOML key                                        | Environment variable                                                         | Default | Description |
|-------------------------------------------------|-------------------------------------------------------------------------------|---------|-------------|
| `plugins.block_access_service.enabled`          | `ROCK_NODE__PLUGINS__BLOCK_ACCESS_SERVICE__ENABLED`                          | `true`  | Enable block-access gRPC service |
| `plugins.block_access_service.grpc_address`     | `ROCK_NODE__PLUGINS__BLOCK_ACCESS_SERVICE__GRPC_ADDRESS`                     | `0.0.0.0` | Bind address |
| `plugins.block_access_service.grpc_port`        | `ROCK_NODE__PLUGINS__BLOCK_ACCESS_SERVICE__GRPC_PORT`                        | `6791`  | Port |

### Server-Status Service

| TOML key                                          | Environment variable                                                            | Default | Description |
|---------------------------------------------------|---------------------------------------------------------------------------------|---------|-------------|
| `plugins.server_status_service.enabled`           | `ROCK_NODE__PLUGINS__SERVER_STATUS_SERVICE__ENABLED`                            | `true`  | Enable server-status gRPC service |
| `plugins.server_status_service.grpc_address`      | `ROCK_NODE__PLUGINS__SERVER_STATUS_SERVICE__GRPC_ADDRESS`                       | `0.0.0.0` | Bind address |
| `plugins.server_status_service.grpc_port`         | `ROCK_NODE__PLUGINS__SERVER_STATUS_SERVICE__GRPC_PORT`                          | `6792`  | Port |

### Query Service

| TOML key                                 | Environment variable                                             | Default | Description |
|------------------------------------------|------------------------------------------------------------------|---------|-------------|
| `plugins.query_service.enabled`          | `ROCK_NODE__PLUGINS__QUERY_SERVICE__ENABLED`                     | `true`  | Enable query gRPC service |
| `plugins.query_service.grpc_address`     | `ROCK_NODE__PLUGINS__QUERY_SERVICE__GRPC_ADDRESS`                | `0.0.0.0` | Bind address |
| `plugins.query_service.grpc_port`        | `ROCK_NODE__PLUGINS__QUERY_SERVICE__GRPC_PORT`                   | `6793`  | Port |

---

## Why double underscores?

Many configuration keys (e.g. `log_level`, `max_concurrent_streams`) themselves contain an underscore. If we used a **single** underscore as the path separator, those names would be split incorrectly when parsing environment variables (e.g. `CORE_LOG_LEVEL` would be interpreted as the path `core.log.level`).
