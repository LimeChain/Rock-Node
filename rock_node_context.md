# Rock Node Project – Context Dump

**Version:** 1.0  
**Date:** June 12, 2025  
**Status:** Final  

---

## 1 Business Context

### 1.1 Background & Problem  
Current Hiero Consensus Nodes upload records to cloud buckets, causing high costs, limited query capabilities, scalability bottlenecks, and centralization risks.

### 1.2 Solution – Rock Node  
A Rust‑based **Block Node** that ingests the new gRPC _Block Stream_, verifies & stores blocks, and exposes optimized APIs for clients.

### 1.3 Value Proposition  
* Off‑loads data‑serving from Consensus Nodes → higher TPS  
* Decentralizes data availability via independently operated nodes  
* Cuts costs for data consumers through direct gRPC instead of S3  
* Adds new services: real‑time streams, State/Block proofs, reconnect relay  
* Opens a market for value‑added data services

---

## 2 Problem Analysis & Opportunities

| Problems Solved | Opportunities Created |
| --- | --- |
| High cloud costs (reader‑pays, LIST ops) | Decentralized data market for operators |
| Complex fragmented streams & sig files | Cryptographic proofs on demand |
| Consensus Nodes overloaded with snapshots/reconnect | Advanced filtered gRPC streams |
| Centralized infrastructure reliance | Plug‑in JSON‑RPC relay, analytics, etc. |

---

## 3 Key Business Processes

1. **Block Ingestion** – Multiple publishers race; Rock Node verifies & ACKs.  
2. **Block Subscription** – Client requests live/historical stream.  
3. **Block Retrieval** – Single historical block fetch.  
4. **State Proof Generation** – Merkle proof for state at block *X*.  
5. **Block Contents Proof** – Inclusion proof for item in block *N*.  
6. **Node Reconnect** – Snapshot + blocks to sync lagging node.

---

## 4 System Architecture (C4)

### 4.1 Containers

| Container | Role |
| --- | --- |
| BlockStreamPublishService | gRPC ingest from Consensus Nodes |
| BlockStreamSubscribeService | gRPC stream to clients |
| BlockAccessService | Single block fetch |
| ProofService | State / Block proofs |
| ReconnectService | Sync lagging nodes |
| Core Messaging | In‑memory Disruptor bus |
| Verifier Service | Validate BlockProof |
| Persistence Service | Tiered storage & archiving |
| Observability Service | Metrics & health endpoints |

### 4.2 Facets

* **Language:** Rust (Tonic for gRPC)  
* **Pattern:** Event‑driven, plugin‑based, Capability Registry  
* **Storage:** RocksDB (hot) + zstd indexed files (cold)  
* **Security:** mTLS, API keys (future), TLS transport  
* **Deployment:** Docker → Kubernetes (Helm), PVC for data

---

## 5 Core Data & Storage

* **Hot Tier:** RocksDB for recent blocks + Merkle state tree  
* **Cold Tier:** Compressed archives with index for random access  
* **VerifiableState:** Custom Merkle tree backed by RocksDB enabling `StateProof`s.

---

## 6 Non‑Functional Highlights

* **Messaging throughput:** ≥ 1 M msgs/s, < 1 µs median latency  
* **Subscribe live latency:** < 100 µs extra  
* **Block access p99:** < 50 ms  
* **Proof generation p99:** < 100 ms  
* **PublishService mTLS support & low overhead**

---

## 7 Architectural Decision Records (ADRs)

| # | Decision | Rationale |
| - | --- | --- |
| ADR‑1 | Plugin‑based core with `Plugin` trait & `AppContext`. | Flexibility, extensibility, parallel dev. |
| ADR‑2 | Central in‑memory Disruptor message bus. | Decoupling & back‑pressure‑free pipeline. |
| ADR‑3 | Two‑tier storage (RocksDB + cold archives). | Balance performance & cost. |
| ADR‑4 | `CapabilityRegistry` for plugin discovery. | Extreme decoupling & composability. |
| ADR‑5 | Custom `VerifiableState` Merkle tree on RocksDB. | Enables cryptographic state proofs. |

---

## 8 Interfaces (gRPC proto methods)

* `publishBlockStream(stream PublishStreamRequest) returns (stream PublishStreamResponse);`
* `subscribeBlockStream(SubscribeStreamRequest) returns (stream SubscribeStreamResponse);`
* `getBlock(BlockRequest) returns (BlockResponse);`
* `getStateProof(StateProofRequest) returns (StateProofResponse);`
* `getBlockContentsProof(BlockContentsProofRequest) returns (BlockContentsProofResponse);`
* `reconnect(ReconnectRequest) returns (stream ReconnectResponse);`

---

## 9 Open Items / Future Work

* Implement zero‑trust security phase (mandatory mTLS + API keys)  
* Expand ProofService for contract‑level proofs (e.g., EVM storage)  
* Optimize Archiver for parallel compression  
* Provide JSON‑RPC relay plugin for EVM compatibility  
* Add correlation IDs for end‑to‑end tracing across event bus

---
