# Benchmark Results: Rust vs. Java Implementation

This benchmark report presents a detailed performance comparison between the Rust and Java implementations of the application. Tests were performed in a containerized environment to measure resource utilization, data processing throughput, and gRPC endpoint latency. The goal is to provide an objective comparison to guide future development and optimization efforts.

## Table of Contents

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

## Overview

We compared two implementations of the same product:

- **Java Implementation**
- **Rust Implementation**

This document aims to:

- Compare initial and under-load resource usage (CPU, Memory, Disk I/O).
- Evaluate the response latency of critical gRPC endpoints.

---

## Environment & Setup

All tests were conducted within identical Docker container configurations on the same host machine to ensure a fair and controlled comparison.

### Infrastructure

- **Operating System:** MacOS 15.5
- **CPU:** Apple M4 Pro
- **Memory:** 24GB
- **Network:** Docker container-based setup

### Docker Resources

- **CPU:** 10 cores
- **Memory:** 20 GB RAM

### Software & Tool Versions

- **Java Block Node:** 0.13.0
- **Rust Block Node:** 0.1.0
- **Docker:** v28.3.0, build 38b7060
- **Java Version (JDK):** v21.0.5 (e.g., OpenJDK 21)
- **Rust Version (rustc):** v1.87.0 (e.g., 1.78.0)
- **Consensus Node Version:** 0.63.9
- **Benchmarking Tools:** System monitoring tools (e.g., docker stats)

---

## Methodology

### 3.1 Initial Resource Usage (Idle)

Before initiating the main workload, each application was started in its container. After a brief stabilization period, a snapshot of the resource consumption was taken using `docker stats` to establish a baseline for idle resource usage.

### 3.2 Block Production & Consumption Test

This test measures the performance of the core data processing pipeline over an approximate 20-minute workload.

1. **Test Start:** The application container is started, and the timer begins.
2. **Block Production:** A consensus node client connects to the application and publishes blocks continuously for 2 minutes.
3. **Block Consumption:** Around the 2 minute of this flow, a consumer client connects and begins fetching blocks sequentially from block 0 to 100,000.
4. **Test End:** The test concludes at the 20ieth minute.

**Metrics Collected:**

- **Latency Time:** The latency for answering quries like getBlock and serverStatus.
- **Average & Peak Resource Usage:** CPU and Memory usage were monitored throughout the entire test.
- **Final Disk Usage:** The total size of all files committed to disk by the application at the end of the test.

### 3.3 gRPC Endpoint Latency

**Endpoints Tested:**

- `getBlock`
- `serverStatus`

**Metrics Collected:**

- **Response Time (avg, p99):** The average and 99th percentile latency for each endpoint.

---

## Results

### 4.1 Initial Resource Usage (Idle)

This table shows the resource consumption of each implementation shortly after startup, before any workload is applied.

| Resource | Java Implementation | Rust Implementation | Notes |
|----------|-------------------|-------------------|-------|
| CPU Usage (avg) | 90 % | [FILL_HERE] % | [FILL_HERE] |
| Memory Usage | 1.06 GB | [FILL_HERE] MB | [FILL_HERE] |

### 4.2 Block Production & Consumption Test

This table shows the performance and resource consumption during the full workload test.

| Metric | Java Implementation | Rust Implementation | Notes |
|--------|-------------------|-------------------|-------|
| CPU Usage (avg) | 46.5 % | [FILL_HERE] % | [FILL_HERE] |
| CPU Peak | 122.94 % | [FILL_HERE] % | [FILL_HERE] |
| Memory Usage (avg) | 2.10GB | [FILL_HERE] MB | [FILL_HERE] |
| Memory Peak | 3.28GB | [FILL_HERE] MB | [FILL_HERE] |

### 4.3 gRPC Endpoint Latency

#### getBlock Latency

| Metric | Java Implementation | Rust Implementation | Notes |
|--------|-------------------|-------------------|-------|
| Avg. Response Time | 40.27 ms | [FILL_HERE] ms | [FILL_HERE] |
| p99 Response Time | 178.69 ms | [FILL_HERE] ms | [FILL_HERE] |

#### serverStatus Latency

| Metric | Java Implementation | Rust Implementation | Notes |
|--------|-------------------|-------------------|-------|
| Avg. Response Time | 8.39 ms | [FILL_HERE] ms | [FILL_HERE] |
| p99 Response Time | 30.04 ms | [FILL_HERE] ms | [FILL_HERE] |

### 4.4 Final Disk Space Usage

This table shows the total disk space consumed by each application's persisted files at the conclusion of the test. Around 587 blocks

| Metric | Java Implementation | Rust Implementation | Notes |
|--------|-------------------|-------------------|-------|
| Total Disk Space | 16,2 MB | [FILL_HERE] MB/GB | [FILL_HERE] |

---

## Analysis & Observations

*(This section will be filled out after the results are collected. Below are template points.)*

TO BE FILLED

---

**Disclaimer:** These benchmark results are specific to the described environment, configuration, and workload. Performance may vary under different conditions, loads, or hardware.

**Last Updated:** 2025-07-09  
**Maintainers:** LimeChain