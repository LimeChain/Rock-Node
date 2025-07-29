# Rock Node

<div align="center">
<img src="assets/logo.jpeg" alt="Rock Node Logo" width="350"/>

<p><em>Resilient as a Rock ⚡</em></p>

[![Rust Version](https://img.shields.io/badge/rust-1.75.0+-blue.svg)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

</div>

## Overview

Rock Node is a high-performance, decentralized data-availability layer for the Hiero network. It provides a robust solution for data availability and verification, enabling efficient block storage and retrieval while maintaining decentralization.

Key features:
- High-performance block ingestion and verification
- Decentralized data availability
- Efficient block storage and retrieval
- Real-time block streaming
- Cryptographic proof generation
- Plugin-based architecture for extensibility

## Prerequisites

Before you begin, ensure you have the following installed:
- Rust 1.75.0 or later
- Cargo (Rust's package manager)
- Git
- Docker (optional, for containerized deployment)

## Getting Started

### Building from Source

1. Clone the repository:
   ```bash
   git clone https://github.com/yourusername/rock-node.git
   cd rock-node
   ```

2. Build the project:
   ```bash
   cargo build --release
   ```

3. Run the application:
   ```bash
   cargo run --release
   ```

### Configuration

Rock-Node ships with a comprehensive default configuration at `config/config.toml`. **Every value can be overridden via environment variables** – perfect for container or cloud deployments.

See the full [Configuration Reference](docs/CONFIGURATION.md) for a table-by-table mapping of TOML keys to their corresponding environment-variable names, default values and descriptions.

Quick override example:

```bash
# run with TRACE log level without touching the TOML file
ROCK_NODE__CORE__LOG_LEVEL=TRACE cargo run
```

### Docker Deployment

To run using Docker:

```bash
docker build -t rock-node .
docker compose up -d
```

## Contributing

We welcome contributions to Rock Node! Please see our [Contributing Guide](CONTRIBUTING.md) for details on how to submit pull requests, report issues, and suggest improvements.

### Development Workflow

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add some amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

## Code of Conduct

Please read our [Code of Conduct](CODE_OF_CONDUCT.md) to keep our community approachable and respectable.

## License

This project is licensed under:
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
