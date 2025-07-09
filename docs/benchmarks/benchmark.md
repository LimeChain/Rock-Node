# 🚀 Benchmark Results: Rust vs. Java Implementation

This benchmark report presents a detailed performance comparison between the Rust and Java implementations of the application. Tests were performed in a containerized environment to measure resource utilization, data processing throughput, and gRPC endpoint latency. The goal is to provide an objective comparison to guide future development and optimization efforts.

## 📋 Table of Contents

1. [Overview](#overview)
2. [Environment & Setup](#environment--setup)
3. [Methodology](#methodology)
   - [3.1 Initial Resource Usage (Idle)](#31-initial-resource-usage-idle)
   - [3.2 Block Production & Consumption Test](#32-block-production--consumption-test)
   - [3.3 gRPC Endpoint Latency](#33-grpc-endpoint-latency)
4. [Results](#results)
   - [4.1 Initial Resource Usage (Idle)](#41-initial-resource-usage-idle)
   - [4.2 Block Production & Consumption Test](#42-block-production--consumption-test)
   - [4.3 gRPC Endpoint Latency](#43-grpc-endpoint-latency)
   - [4.4 Final Disk Space Usage](#44-final-disk-space-usage)
5. [Analysis & Observations](#analysis--observations)

---

## 🎯 Overview

We compared two implementations of the same product:

- ☕ **Java Implementation**
- 🦀 **Rust Implementation**

This document aims to:

- Compare initial and under-load resource usage (CPU, Memory, Disk I/O).
- Evaluate the response latency of critical gRPC endpoints.

---

## ⚙️ Environment & Setup

All tests were conducted within identical Docker container configurations on the same host machine to ensure a fair and controlled comparison.

### 🖥️ Infrastructure

- **Operating System:** MacOS 15.5
- **CPU:** Apple M4 Pro
- **Memory:** 24GB
- **Network:** Docker container-based setup

### 🐳 Docker Resources

- **CPU:** 10 cores
- **Memory:** 20 GB RAM

### 🔧 Software & Tool Versions

- **Java Block Node:** 0.13.0
- **Rust Block Node:** 0.1.0
- **Docker:** v28.3.0, build 38b7060
- **Java Version (JDK):** v21.0.5 (e.g., OpenJDK 21)
- **Rust Version (rustc):** v1.87.0 (e.g., 1.78.0)
- **Consensus Node Version:** 0.63.9
- **Benchmarking Tools:** System monitoring tools (e.g., docker stats)

---

## 📊 Methodology

### 3.1 Initial Resource Usage (Idle)

Before initiating the main workload, each application was started in its container. After a brief stabilization period, a snapshot of the resource consumption was taken using `docker stats` to establish a baseline for idle resource usage.

### 3.2 Block Production & Consumption Test

This test measures the performance of the core data processing pipeline over an approximate 20-minute workload.

1. **Test Start:** The application container is started, and the timer begins.
2. **Block Production:** A consensus node client connects to the application and publishes blocks continuously for 2 minutes.
3. **Block Consumption:** Around the 2-minute mark of this flow, a consumer client connects and begins fetching blocks sequentially from block 0 to 100,000.
4. **Test End:** The test concludes at the 20th minute.

**Metrics Collected:**

- **Latency Time:** The latency for answering queries like `getBlock` and `serverStatus`.
- **Average & Peak Resource Usage:** CPU and Memory usage were monitored throughout the entire test.
- **Final Disk Usage:** The total size of all files committed to disk by the application at the end of the test.

### 3.3 gRPC Endpoint Latency

**Endpoints Tested:**

- `getBlock`
- `serverStatus`

**Metrics Collected:**

- **Response Time (avg, p99):** The average and 99th percentile latency for each endpoint.

---

## 📈 Results

### 4.1 Initial Resource Usage (Idle)

This table shows the resource consumption of each implementation shortly after startup, before any workload is applied.

| Resource | ☕ Java Implementation | 🦀 Rust Implementation | Notes |
|----------|-------------------|-------------------|-------|
| CPU Usage (avg) | 90% | 0.03% | Rust's idle CPU usage is negligible, while Java consumes significant resources even at idle. |
| Memory Usage | 1.06 GB | 4.18 MB | Rust's memory footprint is over 250x smaller than Java's at idle. |

### 4.2 Block Production & Consumption Test

This table shows the performance and resource consumption during the full workload test.

| Metric | ☕ Java Implementation | 🦀 Rust Implementation | Notes |
|--------|-------------------|-------------------|-------|
| CPU Usage (avg) | 46.5% | 0.15% | Under load, Java's average CPU usage is over 300x higher than Rust's. |
| CPU Peak | 122.94% | 10.2% | The peak CPU load for Java is more than 12x higher than for Rust. |
| Memory Usage (avg) | 2.10 GB | 40.3 MB | Rust's average memory usage under load is ~52x lower than Java's. |
| Memory Peak | 3.28 GB | 77.42 MB | Java's peak memory consumption is ~42x higher than Rust's. |

### 4.3 gRPC Endpoint Latency

#### 🔍 getBlock Latency

| Metric | ☕ Java Implementation | 🦀 Rust Implementation | Notes |
|--------|-------------------|-------------------|-------|
| Avg. Response Time | 40.27 ms | 30.22 ms | Rust is ~25% faster on average for block retrieval. |
| p99 Response Time | 178.69 ms | 88.52 ms | Rust's p99 latency is ~50% lower, indicating much better worst-case performance. |

#### 📊 serverStatus Latency

| Metric | ☕ Java Implementation | 🦀 Rust Implementation | Notes |
|--------|-------------------|-------------------|-------|
| Avg. Response Time | 8.39 ms | 11.58 ms | Java shows a slightly lower average response time for status checks. |
| p99 Response Time | 30.04 ms | 44.47 ms | Java demonstrates better worst-case performance for the status endpoint. |

### 4.4 Final Disk Space Usage

This table shows the total disk space consumed by each application's persisted files at the conclusion of the test. Around 587 blocks were processed.

| Metric | ☕ Java Implementation | 🦀 Rust Implementation | Notes |
|--------|-------------------|-------------------|-------|
| Total Disk Space | 16.2 MB | 7.8 MB | Rust uses less than half (~48%) the disk space to store the same amount of data. |

---

## 🔍 Analysis & Observations

The results from this benchmark clearly indicate a significant performance and efficiency gap between the Java and Rust implementations.

### 💾 Resource Utilization

The most striking difference is in resource consumption. The Rust implementation is exceptionally lightweight in comparison to the Java version.

### ⚡ CPU & Memory

At idle, the Java application consumes substantial resources (90% CPU, 1.06 GB RAM), likely due to the JVM overhead, while the Rust application is nearly dormant. This trend continues under load, where Rust's CPU and memory usage are orders of magnitude lower. The peak resource consumption figures further highlight Rust's stability and efficiency, with Java experiencing peaks over 12x higher for CPU and 42x higher for memory.

### 💿 Disk Space

Rust also proves to be more efficient in storage, consuming less than half the disk space of the Java implementation for the same dataset. This suggests a more compact data serialization or storage format, which could lead to significant cost savings at scale.

### 🌐 gRPC Performance & Latency

For the critical `getBlock` operation, the Rust implementation demonstrates superior performance. It not only has a 25% lower average response time but, more importantly, its 99th percentile latency is half that of Java's. This indicates that the Rust service is more consistent and reliable under pressure, avoiding the high-latency outliers seen in the Java version.

Interestingly, the Java implementation showed a slight performance advantage in the `serverStatus` check. While this is a point of data, the `getBlock` endpoint is far more critical to the core functionality of the application, making Rust's advantage there more significant.

### 🎯 Conclusion & Scalability

Based on every key metric—CPU usage, memory consumption, peak stability, disk space efficiency, and critical endpoint latency—the Rust implementation is decisively the more performant and efficient solution. Its low resource footprint would translate directly to lower operational costs in a production environment. Furthermore, its stable, low-latency performance on core operations suggests it would scale far more effectively and reliably as the workload increases.

---

**⚠️ Disclaimer:** These benchmark results are specific to the described environment, configuration, and workload. Performance may vary under different conditions, loads, or hardware.

**📅 Last Updated:** 2025-07-09  
**👥 Maintainers:** LimeChain
