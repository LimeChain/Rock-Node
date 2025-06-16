
# Rock Node – Implementation Tracker ✔️

> Tick `[x]` when a task is complete. Indent levels reflect **Epic → Story/Task**.

---

## 🔵 Epic 1 – Project Bootstrap
- [ ] **E1-S1** Repo & branching rules  
- [ ] **E1-S2** License & GitHub templates  
- [ ] **E1-S3** Cargo workspace scaffold  
- [ ] **E1-S4** Rust toolchain & lint config  
- [ ] **E1-S5** Initial CI pipeline  
- [ ] **E1-S6** Core dependencies pinned  

---

## 🟣 Epic 2 – gRPC API Crate
- [ ] **E2-S1** Copy proto files to `crates/api/proto`  
- [ ] **E2-S2** Create `rock_node_api` crate with deps  
- [ ] **E2-S3** `build.rs` tonic‑build integration  
- [ ] **E2-S4** Re‑export generated modules  
- [ ] **E2-S5** CI `cargo check -p rock_node_api`  
- [ ] **E2-S6** (Opt) Publish docs to GH Pages  

---

## 🟢 Epic 3 – Application Shell & AppContext
- [ ] **E3-S1** `main.rs` with CLI (`clap`)  
- [ ] **E3-S2** Config loader (TOML + overrides)  
- [ ] **E3-S3** Typed `Settings` struct  
- [ ] **E3-S4** Tracing initialization  
- [ ] **E3-S5** `AppContext` (Arc)  
- [ ] **E3-S6** Project‑wide error strategy (`anyhow`)  
- [ ] **E3-S7** Healthcheck flag  

---

## 🟠 Epic 4 – Core Messaging Bus
- [ ] **E4-S1** Define `Event` trait & derive macro  
- [ ] **E4-S2** Implement `RingBuffer<E>` w/ Loom tests  
- [ ] **E4-S3** `Publisher` API & benchmark  
- [ ] **E4-S4** `Subscriber` API w/ Notify  
- [ ] **E4-S5** Backpressure & drop metrics  
- [ ] **E4-S6** Prometheus metrics & tracing spans  
- [ ] **E4-S7** Integrate into `AppContext`  
- [ ] **E4-S8** Micro‑benchmarks documented  

---

## 🟡 Epic 5 – BlockStreamPublishService v0
- [ ] **E5-P1** Scaffold `publish_service` crate  
- [ ] **E5-P2** Wire service into binary flag  
- [ ] **E5-S1** Implement tonic service trait  
- [ ] **E5-S2** Connection state map (`DashMap`)  
- [ ] **E5-S3** Stream ingest → publish events  
- [ ] **E5-S4** ACK on `BlockPersisted` event  
- [ ] **E5-S5** Timeout & error handling  
- [ ] **E5-S6** Metrics & tracing instrumentation  
- [ ] **E5-S7** Example `mock_publisher`  
- [ ] **E5-S8** Load test & report  

---

### Legend
| Color | Epic |
| --- | --- |
| 🔵 | Bootstrap |
| 🟣 | API Crate |
| 🟢 | App Shell |
| 🟠 | Messaging Bus |
| 🟡 | Publish Service |

---

> **Tip for the next LLM:** Mark tasks done by replacing `[ ]` with `[x]`.  
> Eg. `- [x] E1-S1 Repo & branching rules  ✅`

