# Rock Node Publish-Plugin Design

## Table of Contents

1. [Introduction](#1-introduction)
   - [1.1 Purpose](#11-purpose)
   - [1.2 Scope](#12-scope)
2. [High-Level Architecture](#2-high-level-architecture)
3. [Detailed Component Design](#3-detailed-component-design)
   - [3.1 Component Responsibilities](#31-component-responsibilities)
4. [Key Data Flows & State Machine](#4-key-data-flows--state-machine)
   - [4.1 Primary Election Flow](#41-primary-election-flow)
   - [4.2 Primary Publisher Flow](#42-primary-publisher-flow)
5. [Shared State & Concurrency Control](#5-shared-state--concurrency-control)
6. [API Definition](#6-api-definition)
7. [Observability](#7-observability)
   - [7.1 Metrics](#71-metrics)
   - [7.2 Logging](#72-logging)
8. [Future-Proofing and Scalability](#8-future-proofing-and-scalability)
   - [8.1 Eviction of Stale "Winners"](#81-eviction-of-stale-winners)
   - [8.2 Stronger Memory Ordering for State](#82-stronger-memory-ordering-for-state)
   - [8.3 Dynamic Configuration](#83-dynamic-configuration)

---

## 1. Introduction

### 1.1 Purpose

This document provides the software design for the Publish Plugin. This plugin provides a gRPC endpoint for data producers (publishers) to stream block data into the Rock Node. It is designed to be highly concurrent, handling multiple simultaneous publisher connections while ensuring that blocks are processed sequentially, exactly once, and in order. It implements a robust flow-control mechanism to prevent fast publishers from overwhelming the node's persistence layer.

### 1.2 Scope

This document covers the plugin's architecture, including its use of bidirectional gRPC streaming, the session management state machine, the leader-election mechanism for block processing, and the shared state model. It is the canonical reference for the plugin's implementation.

---

## 2. High-Level Architecture (C4 Level 2)

The Publish Plugin exposes a bidirectional gRPC stream. Publishers connect and push BlockItem data to the server, and the server pushes back acknowledgements, flow-control messages, and error notifications. The plugin coordinates with the core messaging bus to ingest blocks and relies on persistence events for its flow-control loop.

```mermaid
graph TD
    subgraph "Publishers"
        direction LR
        PublisherA("Publisher A")
        PublisherB("Publisher B")
    end

    subgraph "Rock Node Boundary"
        direction TB
        
        PublishPlugin["
            **Publish Plugin**
            [Rust Crate]
            ---
            Manages multiple publisher sessions.
            Elects a 'primary' publisher for each block.
            Enforces sequential block processing.
            Provides flow control via explicit ACKs.
        "]

        CoreMessaging("
            <b>Core Messaging Bus</b>
            <br/>[tokio::broadcast]<br/>
            `BlockItemsReceived` channel (write).
            `BlockPersisted` channel (read).
        ")

        CoreCache("
            <b>Block Data Cache</b>
            <br/>[moka::sync::Cache]<br/>
            Temporarily stores block data between plugins.
        ")
        
        PublisherA -- "1. Streams BlockItems" --> PublishPlugin
        PublisherB -- "1. Streams BlockItems" --> PublishPlugin
        
        PublishPlugin -- "2. Sends 'BlockItemsReceived'" --> CoreMessaging
        PublishPlugin -- "3. Puts data in" --> CoreCache
        PublishPlugin -- "4. Subscribes to 'BlockPersisted'" --> CoreMessaging
        
        PublishPlugin -- "5. Streams ACKs/NACKs back" --> PublisherA
        PublishPlugin -- "5. Streams ACKs/NACKs back" --> PublisherB
    end
```

**Diagram 2.1:** Container-level view of the Publish Plugin.

---

## 3. Detailed Component Design (C4 Level 3)

The plugin is composed of a central service implementation that spawns a dedicated SessionManager task for each incoming connection. These sessions coordinate through a SharedState object.

```mermaid
graph TD
    subgraph "External Dependencies"
        AppContext("<b>AppContext</b><br/>Config, Metrics, Core Channels")
    end

    subgraph "Publish Plugin Boundary"
        direction TB
        
        PluginManager("
            <b>PublishPlugin</b>
            <br/>[lib.rs]<br/>
            Implements `Plugin` trait.
            Initializes `SharedState` and gRPC server.
        ")

        ServiceFacade("
            <b>PublishServiceImpl</b>
            <br/>[service.rs]<br/>
            Implements gRPC `BlockStreamPublishService`.
            Accepts new connections and spawns a SessionManager for each.
        ")

        SharedState("
            <b>SharedState</b>
            <br/>[state.rs]<br/>
            `Arc<T>` containing `DashMap` for winner election
            and `AtomicI64` for latest persisted block.
            The central point of coordination.
        ")

        subgraph "Per-Connection Task"
            direction LR
            SessionManager("
                <b>SessionManager</b>
                <br/>[session_manager.rs]<br/>
                Manages the state for one client stream.
                Handles the primary election logic.
                Publishes blocks and waits for ACKs.
            ")
        end
        
        %% Flows
        AppContext -- "Used by" --> PluginManager
        PluginManager -- "Instantiates & Owns" --> SharedState
        PluginManager -- "Starts" --> ServiceFacade

        ServiceFacade -- "Spawns a" --> SessionManager
        SessionManager -- "Reads/Writes" --> SharedState
        SessionManager -- "Uses" --> AppContext
    end
```

**Diagram 3.1:** Internal components of the Publish Plugin.

### 3.1 Component Responsibilities

#### PublishPlugin (lib.rs)

The main plugin entry point. It initializes the singleton `SharedState` instance, retrieves the node's latest known block from the BlockReader to hydrate the state, and starts the main tonic gRPC server, configuring TCP and HTTP/2 keepalives.

#### PublishServiceImpl (service.rs)

The gRPC service implementation. Its sole responsibility is to handle new incoming publisher streams. For each new connection, it increments a metrics gauge and spawns a new asynchronous tokio task to run a SessionManager instance, handing it the client's request stream and the server's response stream handle.

#### SessionManager (session_manager.rs)

The stateful workhorse of the plugin; one instance exists per active connection. It reads incoming `PublishStreamRequest` messages and processes them according to its internal state. It interacts with the `SharedState` to participate in the primary election for a given block. If it becomes primary, it is responsible for collecting all BlockItems for a block, publishing the completed block to the core messaging bus, and waiting for a `BlockPersisted` event as a signal to send an `Acknowledgement` back to the client.

#### SharedState (state.rs)

A thread-safe object shared across all SessionManager instances. It contains:

- **block_winners:** A `DashMap<u64, Uuid>` used for the primary election. It maps a block number to the unique ID of the session that has won the right to publish it.
- **latest_persisted_block:** An `AtomicI64` that holds the highest block number confirmed to be durable. This is used to quickly reject duplicate or out-of-order (future) blocks.

---

## 4. Key Data Flows & State Machine

The core of the plugin is the "primary election" and subsequent state transitions within each SessionManager.

### 4.1 Primary Election Flow

1. A SessionManager receives the first `BlockItem` for block N, which must be a `BlockHeader`.
2. It reads `SharedState::latest_persisted_block` to perform initial validation.
3. If N <= latest_persisted, it is a duplicate. The server sends `EndOfStream(DuplicateBlock)` and closes the connection.
4. If N > latest_persisted + 1, it is a future block. The server sends `EndOfStream(Behind)` and closes the connection.
5. If N == latest_persisted + 1 (the expected block), the session attempts to "win" the election by inserting its unique session ID into `SharedState::block_winners` for key N.
6. The `DashMap::entry().or_insert()` operation atomically guarantees that only one session can be the first to write its ID.

**Case A (Win):** The session's ID was successfully inserted. It transitions its internal state to `Primary`. It can now buffer `BlockItems`.

**Case B (Loss):** The session finds another session's ID already present. It transitions its internal state to `Behind`. It sends a `SkipBlock` message to its client and ignores all subsequent `BlockItems` for block N.

### 4.2 Primary Publisher Flow (State == Primary)

1. The primary SessionManager buffers all incoming `BlockItems` in memory.
2. Upon receiving the final item for the block (the `BlockProof`), it bundles all buffered items into a `Block` protobuf object.
3. It serializes the `Block` and places the bytes into the AppContext's `BlockDataCache`.
4. It sends a `BlockItemsReceived` event containing the block number and cache key to the core message bus.
5. It now synchronously awaits a `BlockPersisted` event for block N from the persistence plugin's notification channel, with a 30-second timeout.

**Case A (ACK Received):** A `BlockPersisted` event for block N arrives in time.

- The SessionManager sends a `BlockAcknowledgement` response to its client.
- It updates `SharedState::latest_persisted_block` to N.
- It removes N from the `SharedState::block_winners` map.
- It resets its internal state to handle the next block (N+1).

**Case B (Timeout):** No `BlockPersisted` event arrives.

- The SessionManager assumes a failure in the persistence layer.
- It sends a `ResendBlock` request to its client, asking it to try sending the entire block again.
- It removes N from the `SharedState::block_winners` map so another session (or the same one) can retry.

---

## 5. Shared State & Concurrency Control

- **Winner Election:** The `DashMap` provides a highly concurrent, lock-free mechanism for the primary election, which is the main point of contention.
- **Idempotency:** The `latest_persisted_block` atomic check provides a fast path for rejecting the vast majority of duplicate blocks without touching the more expensive `DashMap`.
- **Flow Control:** The publish → await ACK → respond loop is the primary mechanism for flow control. A publisher cannot send block N+1 until it has received an `Acknowledgement` for block N, preventing it from running ahead of the persistence layer.

---

## 6. API Definition

The service is defined by a single bidirectional gRPC stream.

**Service:** `BlockStreamPublishService`

**RPC:** `publish_block_stream(stream PublishStreamRequest) returns (stream PublishStreamResponse)`

### Client-to-Server (PublishStreamRequest)

- **BlockItems:** A message containing a list of one or more `BlockItems`. A block stream begins with a `BlockHeader` and ends with a `BlockProof`.
- **EndStream:** A message from the client indicating it is gracefully closing the stream.

### Server-to-Client (PublishStreamResponse)

- **BlockAcknowledgement:** Sent when a block has been successfully received and persisted. The primary mechanism for flow control.
- **ResendBlock:** Sent to the primary if the server timed out waiting for persistence. The client should resend all items for that block.
- **SkipBlock:** Sent to a non-primary publisher to tell it another publisher won the race and it should not send data for the current block.
- **EndOfStream:** Sent by the server when it is terminating the connection due to an unrecoverable error (e.g., duplicate or future block).

---

## 7. Observability

### 7.1 Metrics

#### `active_publish_sessions`
A Gauge tracking the number of currently connected publishers.

#### `publish_items_processed_total`
A Counter for the total number of individual `BlockItems` processed.

#### `publish_blocks_received_total`
A Counter for blocks seen at the header stage.

**Labels:** `status` ("primary", "behind", "duplicate", "future_block").

#### `publish_responses_sent_total`
A Counter for responses sent to clients.

**Labels:** `type` ("Acknowledgement", "ResendBlock", "SkipBlock", etc.).

#### `publish_persistence_duration_seconds`
A Histogram measuring the time from when a primary sends a block for persistence until it gets the ACK.

**Labels:** `outcome` ("acknowledged", "timeout").

#### `blocks_acknowledged`
A Counter tracking the number of successfully persisted blocks via this plugin.

### 7.2 Logging

- **INFO:** New connections, session state transitions (becoming primary), successful block publications, and connection terminations are logged with the unique `session_id`.
- **WARN:** Duplicate/future blocks, persistence timeouts, and failures to send messages to a client (likely due to a disconnected client) are logged.
- **ERROR:** Critical failures like the gRPC server failing to start or failure to send an event on a core channel.

---

## 8. Future-Proofing and Scalability

### 8.1 Eviction of Stale "Winners"

**Problem:** If a publisher becomes primary for a block N and then crashes or disconnects without sending the full block, its session ID will remain in the `block_winners` map indefinitely. This will stall the node, as it will never be able to process block N.

**Recommended Solution:**

1. Change the `SharedState::block_winners` value from `Uuid` to a struct: `(Uuid, std::time::Instant)`.
2. When a session becomes primary, it stores its ID and the current time.
3. When a new session for block N finds an existing entry, it must check the `Instant`. If `Instant::elapsed()` is greater than a configurable timeout (e.g., 60 seconds), the new session can consider the previous winner "stale."
4. The new session should then use an atomic compare-and-swap operation on the `DashMap` entry to replace the stale winner with itself. This prevents a thundering herd of new sessions from all trying to replace the stale one simultaneously.

### 8.2 Stronger Memory Ordering for State

**Problem:** The `SharedState::latest_persisted_block` uses `Ordering::Relaxed`. While performant, this provides no causal guarantees between the update of the block number and other memory operations. A subtle race condition could occur where a thread sees the updated block number but not other state changes that should have happened before it.

**Recommended Solution:** Change the memory ordering for the atomic load and store operations to `Ordering::SeqCst` (Sequentially Consistent). This is the strongest guarantee and ensures a total global order of operations on the atomic variable, preventing reordering bugs. The performance cost for this variable, which is updated infrequently (once per block), is negligible and the safety gain is significant.

### 8.3 Dynamic Configuration

**Problem:** Key operational parameters, like the 30-second persistence timeout, are currently hardcoded as constants. This lacks flexibility for different deployment environments (e.g., development vs. production).

**Recommended Solution:** Move these parameters into the plugin's configuration struct in `rock-node-core`.

- `persistence_ack_timeout_seconds`: The timeout for awaiting a persistence event.
- `stale_winner_timeout_seconds`: The timeout for the stale winner eviction mechanism described above.

This allows operators to tune the system's behavior without recompiling the code.