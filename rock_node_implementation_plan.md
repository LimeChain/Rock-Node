
# Rock Node – Detailed Implementation Plan (Epics 1‑5)

**Last updated:** 12 Jun 2025  
**Author:** Architecture team

---

## Overview

This backlog breaks down the first five epics—project bootstrap through the initial BlockStreamPublishService—into atomic stories ready for Jira or GitHub Projects.  Each story has a clear *Definition of Done* (DoD), and every epic includes timing, risk notes, and dependencies.

---

## Epic 1 – Project Bootstrap (“Zero‑to‑One”)

| **ID** | **Story / Task** | **Detail / Acceptance Criteria** | **DoD** |
| --- | --- | --- | --- |
| **E1‑S1** | **Repo & branching** | • Create org repo<br>• Default branch `main`<br>• PR‑only merges with squash; CI required | Protect rules present & enforced |
| **E1‑S2** | **License & templates** | Add `LICENSE` (MIT+Apache‑2), issue + PR templates, CODE_OF_CONDUCT | Templates render in new PR |
| **E1‑S3** | **Cargo workspace scaffold** | Root `Cargo.toml` (workspace), dirs: `/app/rock_node`, `/crates/core`, `/crates/common` | `cargo check --workspace` passes |
| **E1‑S4** | **Toolchain & lint** | `rust-toolchain.toml` pin (1.78), `rustfmt.toml`, `clippy.toml`, `cargo-deny` audit config | CI shows zero lint/audit errors |
| **E1‑S5** | **CI pipeline** | GitHub Actions matrix (linux, mac); steps: fmt → clippy → tests → audit → coverage | Status badge in `README.md` |
| **E1‑S6** | **Core deps** | Declare in `[workspace.dependencies]`: `tokio`, `tracing`, `anyhow`, `thiserror` | No duplicate crate versions |

**Time‑box:** 3 dev‑days   
**Risk:** Minimal (developer‑experience only)

---

## Epic 2 – gRPC API Crate

| **ID** | **Story** | **Detail** | **DoD** |
| --- | --- | --- | --- |
| **E2‑S1** | Copy proto files | Place under `crates/api/proto/...` mirroring package names | Files tracked in Git |
| **E2‑S2** | `rock_node_api` crate | Runtime deps `tonic`, `prost`; build‑deps `tonic-build`, `prost-build` | `cargo build -p rock_node_api` ok |
| **E2‑S3** | `build.rs` codegen | Use `tonic_build::configure().build_server(true)`; re‑run on proto change | Regenerates on `touch .proto` |
| **E2‑S4** | Re‑exports | `lib.rs` re‑publishes generated modules & service traits | Doc test compiles |
| **E2‑S5** | CI check | Add job `cargo check -p rock_node_api` | CI green |
| **E2‑S6** | Docs (optional) | `cargo doc --no-deps`, publish via GH Pages | Docs accessible |

**Time‑box:** 2 dev‑days

---

## Epic 3 – Application Shell & `AppContext`

| **ID** | **Story** | **Detail** | **DoD** |
| --- | --- | --- | --- |
| **E3‑S1** | `main.rs` entry | CLI with `clap`: `--config`, `--version` | `rock_node --version` prints SHA |
| **E3‑S2** | Config loader | Using `config` crate; supports `config.local.toml` overrides | Unit tests for merge order |
| **E3‑S3** | Strongly‑typed `Settings` | Derive `Deserialize`, validate invariants | `Settings::load()` returns valid |
| **E3‑S4** | Tracing init | JSON logs with env filter; starts before anything else | First log line visible |
| **E3‑S5** | `AppContext` struct | Holds `Settings`, shutdown chan, etc.; wrapped in `Arc` | Borrow checker clean |
| **E3‑S6** | Error strategy | Use `anyhow`; top‑level `main()` converts to exit code 1 | No `unwrap()` in binary |
| **E3‑S7** | Healthcheck CLI | `--healthcheck` loads config then exits 0 | Docs for K8s probes |

**Time‑box:** 4 dev‑days

---

## Epic 4 – Core Messaging Bus (Disruptor‑lite)

### Design Highlights

* Pre‑allocated **ring buffer** (power‑of‑2 size)  
* Hot‑path lock‑free via `AtomicU64` sequence counters  
* Tokio‑native `Notify` for subscriber wake‑up  
* Independent subscriber cursors → slow consumers don’t block fast ones

### Stories

| ID | Story | DoD |
| --- | --- |
| **E4‑S1** – Event trait & macro | `pub trait Event: Send + Sync` + derive macro |
| **E4‑S2** – `RingBuffer<E>` | `publish`, `get_mut(idx)`, `capacity`; Loom tests |
| **E4‑S3** – `Publisher` API | p50 publish < 250 ns (criterion) |
| **E4‑S4** – `Subscriber` API | Two concurrent subs integration test |
| **E4‑S5** – Backpressure policy | Drops counted; metric `subscriber_lag` |
| **E4‑S6** – Metrics & tracing | Prom scrape endpoint |
| **E4‑S7** – AppContext integration | `ctx.messaging().publisher()` returns handle |
| **E4‑S8** – Micro‑benchmarks | Document throughput ≥ 1 M events/s |

**Time‑box:** 8 dev‑days (2 devs)  
**Risk:** Use of `unsafe`; mitigated via code review + Loom.

---

## Epic 5 – BlockStreamPublishService v0 (“Happy Path”)

### Preparatory

| ID | Story | DoD |
| --- | --- |
| **E5‑P1** – New crate `publish_service` | Compiles empty |
| **E5‑P2** – Binary wiring | `--enable-publish` starts gRPC server |

### Implementation

| ID | Story | Detail / Acceptance | DoD |
| --- | --- | --- | --- |
| **E5‑S1** | Implement tonic trait | Struct `PublishSvc` with `AppContext` ref | gRPC server runs |
| **E5‑S2** | Connection map | `DashMap<PeerId, ConnState>` lifecycle | Leaks none |
| **E5‑S3** | Stream ingest | Convert `BlockItemSet` → event; publish to bus | End‑to‑end test passes |
| **E5‑S4** | ACK on `BlockPersisted` | Subscriber listens; sends `BlockAcknowledgement` | ACK arrives < 5 ms |
| **E5‑S5** | Error / timeout | Idle > 30 s → finishes with `Status::Cancelled` | Unit test |
| **E5‑S6** | Metrics / tracing | Counters for bytes, blocks, acks; latency histogram | `/metrics` includes them |
| **E5‑S7** | Example mock publisher | `cargo run --example mock_publisher` | Docs compile |
| **E5‑S8** | Load test | ghz: 1 k blocks/s stream, CPU < 100 %, ingest p50 < 500 µs | Report in repo |

**Time‑box:** 10 dev‑days (2 devs)  
**Deferred:** Multi‑publisher race (`SkipBlock`).

---

## Cross‑Epic Criteria

* **Ready:** Story has AC, no open design questions, CI tasks identified  
* **Done:** Tests ≥ 80 % for crate, `clippy` clean, docs updated, pipelines green, reviewed

---

## Indicative Timeline

| Week | Focus |
| --- | --- |
| 1 | Epic 1 |
| 2 | Epic 2 & 50 % Epic 3 |
| 3 | Finish Epic 3, begin Epic 4 |
| 4‑5 | Complete Epic 4 |
| 6‑7 | Epic 5 |
| 8 | Buffer / retro / plan next epics |

---

## Tooling Matrix

| Area | Choice |
| --- | --- |
| Runtime | `tokio` 1.x |
| gRPC | `tonic` 0.11 |
| Config | `config` crate (TOML) |
| Logging | `tracing` + `tracing-subscriber` |
| Lint | `rustfmt`, `clippy` |
| CI | GitHub Actions |
| Bench | `criterion`, `loom` |
| Container | Multi‑stage Docker → scratch |
| Observability | OpenTelemetry OTLP → Grafana Tempo/Loki |

---

## Next Planned Epics

1. **Multi‑Publisher Race Logic**  
2. **Verifier Service** (BLS aggregate sig)  
3. **Persistence Service** (RocksDB hot tier)  
4. **BlockStreamSubscribeService**  
5. **Nightly docker‑compose testnet**

> *End of implementation plan (phase I).*  
