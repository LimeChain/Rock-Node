"# Rock Node Known Issues

This document lists potential issues identified during a codebase review. These are not critical bugs but areas for improvement to enhance reliability, performance, and security. Each issue includes a description, evidence from the code or docs, impact, and recommendations.

## 1. Widespread Use of `unwrap()` and `expect()` in Production Code

### Description
Methods like `unwrap()` and `expect()` assume operations succeed and panic on failure, which can lead to application crashes. Proper error handling would make the code more robust.

### Evidence
- Instances found in files such as `crates/rock-node-block-access-plugin/src/service.rs` (e.g., lines 343, 350), `crates/rock-node-subscriber-plugin/src/session.rs` (e.g., lines 34, 226), and `crates/rock-node-persistence-plugin/src/lib.rs` (e.g., lines 99, 182).
- These occur in critical paths like gRPC handlers and event loops.

### Impact
High risk of runtime crashes, especially under error conditions like network issues or invalid data, reducing uptime in a node application.

### Recommendation
Replace with explicit error handling using `?`, `match`, or `Result`. Prioritize high-traffic areas like plugin starts and database interactions.

## 2. Unresolved Race Condition in Subscriber Plugin

### Description
A race condition exists where a block might be persisted after checking the persistence layer but before subscribing to the event bus, causing the subscriber to miss the event and stall.

### Evidence
- Documented in `docs/subscriber/design-doc.md` (section 8.1) with a recommended fix (re-check after subscribing).
- Code in `77:120:crates/rock-node-subscriber-plugin/src/session.rs` (wait_for_new_block function) subscribes and loops without the re-check.

### Impact
Stalled subscriber sessions under high load or variable timing, leading to poor client experience and potential data loss.

### Recommendation
Implement the re-check as per the design doc. Reference [PR #62](https://github.com/LimeChain/Rock-Node/pull/62) for context.

## 3. No Eviction of Stale \"Winners\" in Publisher Plugin

### Description
If a publisher becomes primary but disconnects before sending full data, its entry persists in shared state, blocking other publishers and potentially stalling the node.

### Evidence
- Documented in `docs/publisher/design-doc.md` (section 8.1) with a suggestion for timestamps and timeouts.
- Code in `1:41:crates/rock-node-publish-plugin/src/state.rs` uses a simple DashMap without eviction; entries are only removed post-persistence.

### Impact
Could halt block processing with unreliable publishers, causing node unavailability.

### Recommendation
Add timestamp checks and atomic eviction as suggested.

## 4. Lack of Data Compaction in Persistence Plugin

### Description
Cold storage accumulates small chunk files without merging or re-compressing, leading to filesystem bloat and performance degradation over time.

### Evidence
- Recommended in `docs/persistence/design-doc.md` (section 6.2.1) for long-term scalability.
- Code in `1:122:crates/rock-node-persistence-plugin/src/cold_storage/writer.rs` and `1:199:crates/rock-node-persistence-plugin/src/cold_storage/reader.rs` writes new chunks without compaction; archiver moves batches but doesn't merge.

### Impact
Manageability issues and higher storage costs in long-running nodes.

### Recommendation
Implement a background task for compaction and re-compression. See [PR #65](https://github.com/LimeChain/Rock-Node/pull/65) for related optimizations.

## 5. Disabled Plugins in Configuration

### Description
Key plugins like state management and verification are disabled by default, potentially leaving the node without essential features like verifiable state or block validation.

### Evidence
- In `config/config.toml` (lines 27-28, 40-41), set to `enabled = false`.
- Design docs (e.g., `docs/state-managment/design-doc.md`) highlight their importance.

### Impact
Incomplete functionality or security risks if unverified blocks are processed.

### Recommendation
Enable if required, or document rationale for default disablement. Reference [PR #76](https://github.com/LimeChain/Rock-Node/pull/76) for state management.

## 6. No Security Vulnerability Scanning

### Description
The project doesn't check for known vulnerabilities in dependencies, which is crucial for a networked app.

### Evidence
- Attempted `cargo audit` failed (not installed). Rust crates like `tokio` and `tonic` can have CVEs.

### Impact
Risk of exploits from vulnerable dependencies.

### Recommendation
Install via `cargo install cargo-audit` and integrate into CI. Run regularly.
