# Rock Node Documentation

Welcome to the Rock Node documentation. This directory contains comprehensive design documents and technical specifications for the Rock Node blockchain infrastructure platform.

## Overview

Rock Node is a high-performance, modular blockchain node implementation built in Rust. It provides a robust foundation for blockchain data storage, retrieval, and distribution through a plugin-based architecture. The system is designed for scalability, reliability, and operational excellence.

## Architecture

Rock Node follows a modular plugin architecture where each component is designed as a separate plugin that can be enabled or disabled based on deployment requirements. The core system provides:

- **Plugin Management**: Dynamic loading and lifecycle management of plugins
- **Event Bus**: Asynchronous communication between components via `tokio::broadcast`
- **Configuration Management**: Centralized configuration with environment-specific overrides
- **Observability**: Comprehensive metrics, logging, and monitoring capabilities
- **Data Persistence**: Robust block storage with hot and cold tier support

## Core Plugins

### 🔗 [Block Access Plugin](block-access/design-doc.md)
**Purpose**: Public-facing gRPC API for blockchain data retrieval

The Block Access Plugin provides a simple, robust, and performant access layer to blocks stored by the Persistence Plugin. It exposes a gRPC API that allows external clients and other Rock Node operators to query and retrieve blockchain data.

**Key Features**:
- Single gRPC endpoint for block retrieval
- Support for specific block numbers and latest block requests
- Comprehensive error handling and status reporting
- Prometheus metrics for monitoring and alerting

### 📊 [Server Status Plugin](server-status/design-doc.md)
**Purpose**: Health and status monitoring endpoint

The Server Status Plugin provides a lightweight gRPC service for health checks and status monitoring. It offers a fast and efficient way for clients, operators, and automated monitoring systems to determine the range of block data currently available on the node.

**Key Features**:
- Simple health check endpoint
- Block range reporting (earliest and latest available blocks)
- Low-latency responses for load balancer integration
- Operational metrics for monitoring

### 📤 [Publisher Plugin](publisher/design-doc.md)
**Purpose**: Block data ingestion and streaming

The Publisher Plugin provides a gRPC endpoint for data producers to stream block data into the Rock Node. It handles multiple simultaneous publisher connections while ensuring blocks are processed sequentially, exactly once, and in order.

**Key Features**:
- Bidirectional gRPC streaming
- Primary election mechanism for block processing
- Flow control to prevent overwhelming the persistence layer
- Session management with robust error handling
- Leader election for concurrent publishers

### 📥 [Subscriber Plugin](subscriber/design-doc.md)
**Purpose**: Block data streaming and subscription service

The Subscriber Plugin exposes a public-facing gRPC endpoint that allows clients to subscribe to streams of block data. It serves both finite historical ranges of blocks and open-ended "live" streams that receive new blocks as soon as they are persisted.

**Key Features**:
- Server-side streaming gRPC implementation
- Historical block range streaming
- Live block streaming with event-driven updates
- Session management for multiple concurrent subscribers
- Comprehensive error handling and status reporting

### 💾 [Persistence Plugin](persistence/design-doc.md)
**Purpose**: Core data storage and management

The Persistence Plugin is the authoritative source for block data storage and retrieval. It provides the `BlockReader` trait that other plugins depend on for accessing blockchain data.

**Key Features**:
- Hot and cold storage tier management
- Block data compression and archiving
- Efficient block retrieval and caching
- Data integrity and consistency guarantees
- Storage lifecycle management

## Supporting Documentation

### 🔧 [Protobufs Compilation](protobufs/protobufs-compilation.md)
Technical guide for Protocol Buffer compilation and code generation used across all gRPC services in the Rock Node ecosystem.

## Quick Start

### Prerequisites
- Rust 1.70+ 
- Docker (for containerized deployment)
- PostgreSQL (for persistence layer)

### Configuration
Rock Node uses a centralized configuration system. Key configuration areas include:

- **Plugin Enablement**: Enable/disable specific plugins based on deployment needs
- **Network Settings**: gRPC listener addresses and ports
- **Persistence**: Database connection strings and storage paths
- **Observability**: Metrics endpoints and logging levels

### Deployment
```bash
# Build the project
cargo build --release

# Run with default configuration
./target/release/rock-node

# Run with custom configuration
ROCK_NODE_CONFIG=/path/to/config.toml ./target/release/rock-node
```

## Development

### Plugin Development
Each plugin follows a consistent structure:
- `lib.rs`: Plugin entry point implementing the `Plugin` trait
- `service.rs`: gRPC service implementation (if applicable)
- `state.rs`: Shared state management (if applicable)
- `error.rs`: Error handling and status mapping

### Testing
```bash
# Run all tests
cargo test

# Run tests for a specific plugin
cargo test -p rock-node-block-access-plugin

# Run integration tests
cargo test --test integration
```

## Monitoring and Observability

Rock Node provides comprehensive observability through:

- **Prometheus Metrics**: Detailed metrics for each plugin
- **Structured Logging**: Using the `tracing` library
- **Health Checks**: Built-in health check endpoints
- **Performance Monitoring**: Request duration and throughput metrics
