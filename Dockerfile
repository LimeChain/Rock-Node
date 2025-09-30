# ====================================================================================
# Stage 1: Builder
# ====================================================================================
FROM rust:1.88.0-bullseye AS builder

# Install system dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    protobuf-compiler \
    libclang-dev \
    git \
    cmake \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY . /app

# Use `cargo install` to build the binary and place it in a predictable location.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo install --path app/rock-node --root /dist --locked

# ====================================================================================
# Stage 2: Final Image
# ====================================================================================
FROM gcr.io/distroless/cc-debian12@sha256:b225b1942acac9873950e3982300fc2393f6073c75db7dc1e4825c6116ca3892

WORKDIR /app
USER nonroot

# Only copy the binary. The config file will be provided by a volume mount.
COPY --from=builder --chown=nonroot:nonroot /dist/bin/rock-node /app/rock-node

# This entrypoint passes the config path argument to our application.
# The path `/config/config.toml` will be created by the docker-compose volume mount.
ENTRYPOINT ["/app/rock-node", "--config-path", "/config/config.toml"]
