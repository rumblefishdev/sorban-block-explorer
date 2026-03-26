---
url: 'https://chrisebert.net/comparing-aws-lambda-arm64-vs-x86_64-performance-across-multiple-runtimes-in-late-2025/'
title: 'Comparing AWS Lambda Arm64 vs x86_64 Performance Across Multiple Runtimes in Late 2025'
author: 'Chris Ebert'
date: '2025-11-24'
fetched_date: 2026-03-26
task_id: '0004'
overwritten: false
image_count: 9
images:
  - original_url: 'https://chrisebert.net/content/images/size/w2000/2025/11/Generated-Image-November-21--2025---10_34PM-1.jpeg'
    local_path: 'images/lambda-cold-start-arm-benchmarks/img_1.jpeg'
    alt: "Header illustration labeled 'AWS Lambda Benchmarking' showing a blue x86_64 processor chip on the left and an orange arm64 processor chip on the right, connected by a bidirectional arrow, with Node.js, Python, and Rust runtime logos"
  - original_url: 'https://chrisebert.net/content/images/2025/11/memory-scaling-warm-1.png'
    local_path: 'images/lambda-cold-start-arm-benchmarks/img_2.png'
    alt: 'Line chart showing CPU-intensive warm start duration (ms) vs memory configuration (MB) across all runtimes and architectures. Rust arm64 and x86 lines sit near the bottom well below 500ms, Python lines in the 250-400ms range, Node.js 20 and 22 lines significantly higher around 1200-1600ms. All runtimes improve as memory increases from 128MB to 2048MB.'
  - original_url: 'https://chrisebert.net/content/images/2025/11/python-comparison-warm.png'
    local_path: 'images/lambda-cold-start-arm-benchmarks/img_3.png'
    alt: 'Line chart comparing Python 3.11, 3.12, 3.13, 3.14 warm start durations on arm64 vs x86_64 for CPU-intensive workloads across memory configurations. Python 3.11 arm64 consistently sits lowest, with newer versions showing 9-15% higher execution times.'
  - original_url: 'https://chrisebert.net/content/images/2025/11/nodejs-comparison-warm.png'
    local_path: 'images/lambda-cold-start-arm-benchmarks/img_4.png'
    alt: 'Line chart comparing Node.js 20 vs Node.js 22 warm start durations on arm64 vs x86_64 for CPU-intensive workloads. Node.js 22 arm64 is consistently the fastest, with Node.js 20 x86_64 the slowest; the gap widens slightly at higher memory configurations.'
  - original_url: 'https://chrisebert.net/content/images/2025/11/runtime-family-p99-warm.png'
    local_path: 'images/lambda-cold-start-arm-benchmarks/img_5.png'
    alt: 'Grouped bar chart showing P99 duration for each runtime family (Rust, Python, Node.js) at 2048MB for CPU-intensive warm starts. Rust bars are near zero compared to Python (300ms range) and Node.js (1500ms range). arm64 bars are consistently shorter than x86_64 bars within each group.'
  - original_url: 'https://chrisebert.net/content/images/2025/11/nodejs-rust-comparison-warm.png'
    local_path: 'images/lambda-cold-start-arm-benchmarks/img_6.png'
    alt: 'Line chart directly comparing Rust arm64/x86 vs Node.js 22 arm64/x86 CPU-intensive warm start durations across memory configurations. Rust lines cluster near the bottom below 200ms; Node.js lines are 6-8x higher in the 1200-1600ms range.'
  - original_url: 'https://chrisebert.net/content/images/2025/11/cost-savings-warm-3.png'
    local_path: 'images/lambda-cold-start-arm-benchmarks/img_7.png'
    alt: 'Bar chart showing percentage cost savings of arm64 vs x86_64 for CPU-intensive workloads, broken down by runtime (Rust, Python 3.11-3.14, Node.js 20/22) and memory configuration. Savings range from approximately 7% to 38%, with Rust showing the widest range.'
  - original_url: 'https://chrisebert.net/content/images/2025/11/cost-vs-performance-warm.png'
    local_path: 'images/lambda-cold-start-arm-benchmarks/img_8.png'
    alt: 'Scatter plot of cost (x-axis, USD per million invocations) vs performance/duration (y-axis, ms) for CPU-intensive workloads at 2048MB. arm64 data points cluster in the lower-left (faster and cheaper) compared to x86_64 points in the upper-right. Rust arm64 is the closest to the origin.'
  - original_url: 'https://chrisebert.net/content/images/2025/12/rust-sse-optimization-comparison.png'
    local_path: 'images/lambda-cold-start-arm-benchmarks/img_9.png'
    alt: "Bar chart comparing Rust CPU-intensive performance before and after enabling ARM assembly-optimized SHA-256 hashing. The 'after' bars on arm64 drop from approximately 163ms to 35ms at 2048MB, a 4-5x improvement. x86 bars remain unchanged around 147ms, making arm64 now 4x faster than x86."
---

# Comparing AWS Lambda Arm64 vs x86_64 Performance Across Multiple Runtimes in Late 2025

![Header illustration labeled 'AWS Lambda Benchmarking' showing a blue x86_64 processor chip on the left and an orange arm64 processor chip on the right, connected by a bidirectional arrow, with Node.js, Python, and Rust runtime logos](images/lambda-cold-start-arm-benchmarks/img_1.jpeg)

## Update

This article was initially published November 24th. The original takeaways remain valid, but subsequent optimization enabled dramatically faster Rust performance on Arm. After publishing, [Khawaja Shams](https://github.com/khawajashams) suggested enabling assembly-optimized SHA-256 hashing in the Rust workload. Enabling the `asm` feature on the `sha2` crate resulted in a **4-5x performance improvement for Rust on arm64** in CPU-intensive workloads. Arm64 now completes SHA-256 benchmarking in ~35ms at 2048MB versus ~152ms for x86, making the ARM advantage even more pronounced than initially reported. See the [December 2025 results](https://github.com/cebert/aws-lambda-performance-benchmarks/tree/main/published-results/december-2025) for details.

## Introduction

AWS Lambda originally supported only x86_64-based compute. In 2021, AWS introduced arm64-based Graviton processor support, marketed as offering equivalent or superior performance at lower cost with reduced environmental impact.

In October 2023, AWS published ["Comparing AWS Lambda Arm vs. x86 Performance, Cost, and Analysis."](https://aws.amazon.com/blogs/apn/comparing-aws-lambda-arm-vs-x86-performance-cost-and-analysis-2/) Nearly two years later, updated benchmarks remain scarce across both official AWS channels and community sources. This motivated developing a contemporary benchmark applying similar methodology.

The author expected arm64 to demonstrate superior performance and Rust to show the strongest runtime performance, but sought empirical evidence. The resulting benchmark tests Lambda functions across both architectures using CPU-intensive, memory-intensive, and light workloads with actively supported Node.js, Rust, and Python runtimes.

This project is fully open source on GitHub at [aws-lambda-performance-benchmarks](https://github.com/cebert/aws-lambda-performance-benchmarks), enabling replication, extension, or adaptation to specific workloads.

> **Note:** This benchmark includes the officially supported Rust runtime (GA November 14, 2025) and Python 3.14 runtime (GA November 18, 2025).

## TLDR: The Winners

The benchmark ran several times in the `us-east-2` (Ohio) region, yielding consistent results. The shared data comes from the most recent run, testing 42 Lambda functions (7 runtimes × 2 architectures × 3 workloads). After collecting samples, outliers were removed using statistical techniques, and mean, median, and P50/P90/P95/P99 percentiles were calculated.

- **Performance champion:** Rust on arm64 is the most performant and cost-efficient combination overall. Occasional instances exist where x86_64 Rust marginally exceeds arm64, but with 20% cost discount, arm still wins efficiency-wise.
- **Python:** Python 3.11 on arm64 slightly outperformed newer Python runtimes in testing (matching other public benchmarks).
- **Node.js:** Node.js 22 on arm64 consistently outpaced Node.js 20 on x86_64, offering approximately 15-20% speedup through architecture switching alone.
- **Cost:** Across the board, arm64 delivered roughly 30-40% lower compute costs with equal or better performance compared to x86_64. Unless library incompatibility exists or unique workload characteristics apply, arm represents a sound default architecture choice.

## Benchmark Methodology

The goal involved creating an updated benchmark mirroring the [2023 AWS Lambda benchmark blog post](https://aws.amazon.com/blogs/apn/comparing-aws-lambda-arm-vs-x86-performance-cost-and-analysis-2/). Unable to locate the original code, a new similar benchmark was built from scratch.

The existing AWS benchmark employed three workload types, which were also adopted:

- **Light**: Lightweight but realistic workload.
- **CPU-intensive**: Compute utilization stressing workload.
- **Memory-intensive**: Memory utilization stressing workload.

AWS [Lambda allocates CPU power proportionally to configured memory](https://docs.aws.amazon.com/lambda/latest/dg/configuration-memory.html). Achieving **1 full vCPU requires allocating 1,769 MB of memory to a Lambda**. For single-threaded workloads, diminishing returns occur with memory exceeding this threshold, since single-threaded Lambdas cannot utilize more than one vCPU.

### Workloads

- **Light**: Realistic workload utilizing DynamoDB batch write (5 items) followed by batch read (5 items). Testing includes AWS SDK overhead, serialization/deserialization, and network I/O latency with minimal compute.
- **CPU-intensive**: Performs 500,000 SHA-256 cryptographic hashing iterations in a tight loop. Pure compute workload with no AWS SDK dependencies, designed to stress CPU performance and measure single-threaded execution speed.
- **Memory-intensive**: Allocates and sorts a 100 MB array using native 64-bit types, stressing memory bandwidth and CPU together.

### Runtimes Benchmarked (November 2025)

| Lambda Runtime | Release Date      |
| -------------- | ----------------- |
| Node.js 20     | November 15, 2023 |
| Node.js 22     | November 22, 2024 |
| Python 3.11    | July 27, 2023     |
| Python 3.12    | December 14, 2023 |
| Python 3.13    | November 13, 2024 |
| Python 3.14    | November 18, 2025 |
| Rust           | November 14, 2025 |

### Memory Configurations

| Workload Type    | Memory Configurations (MB)                         | Total Configs |
| ---------------- | -------------------------------------------------- | ------------- |
| Light            | 128, 256, 512, 1024, 1769, 2048                    | 6             |
| CPU-intensive    | 128, 256, 512, 1024, 1769, 2048                    | 6             |
| Memory-intensive | 128, 256, 512, 1024, 1769, 2048, 4096, 8192, 10240 | 9             |

### Sampling

For each configuration: 125 cold invocations and 500 warm invocations (625 total) across 294 unique configurations = **183,750 Lambda invocations per test run**.

## Results Overview

- **arm64 wins on cost in every scenario**
- **Rust dramatically outperforms interpreted runtimes** — 8x faster than Node.js, 2x faster than Python; cold starts favor ARM64 with 13-24% faster initialization
- **Node.js 22 beats Node.js 20** — 8-11% faster execution across the board
- **Python 3.11 is the fastest Python** — 9-15% faster than 3.12, 3.13, and 3.14

## CPU-Intensive Workload Results

### Warm Start Performance

![Line chart showing CPU-intensive warm start duration vs memory configuration across all runtimes and architectures. Rust near bottom, Python in middle, Node.js highest.](images/lambda-cold-start-arm-benchmarks/img_2.png)

| Runtime     | arm64 @2048MB | x86 @2048MB |
| ----------- | ------------- | ----------- |
| **Rust**    | 163ms         | 147ms       |
| Python 3.11 | 263ms         | 341ms       |
| Python 3.14 | 287ms         | 358ms       |
| Node.js 22  | 1,260ms       | 1,384ms     |
| Node.js 20  | 1,377ms       | 1,549ms     |

Rust is **8x faster than Node.js** and nearly **twice as fast as Python** for this compute-heavy workload.

### Python Version Comparison

![Line chart comparing Python 3.11, 3.12, 3.13, 3.14 warm start durations. Python 3.11 arm64 consistently lowest.](images/lambda-cold-start-arm-benchmarks/img_3.png)

Python 3.11 consistently outperformed newer versions by 9-15%.

### Node.js Version Comparison

![Line chart comparing Node.js 20 vs 22 on arm64 vs x86. Node.js 22 arm64 is fastest across all memory configurations.](images/lambda-cold-start-arm-benchmarks/img_4.png)

Node.js 22 showed **8-11% faster execution** than Node.js 20 across memory configurations. Upgrading from Node.js 20 x86 to Node.js 22 arm64 delivers approximately **18% performance improvement at no additional cost**.

### P99 Latency

![Grouped bar chart showing P99 duration by runtime family for CPU-intensive warm starts. Rust bars near zero; Python around 300ms; Node.js around 1500ms. arm64 consistently shorter than x86.](images/lambda-cold-start-arm-benchmarks/img_5.png)

![Line chart directly comparing Rust arm64/x86 vs Node.js 22 arm64/x86. Rust lines cluster below 200ms; Node.js lines 6-8x higher.](images/lambda-cold-start-arm-benchmarks/img_6.png)

### Cost Analysis

![Bar chart showing percentage cost savings of arm64 vs x86_64 for CPU-intensive workloads by runtime. Savings range 7-38%.](images/lambda-cold-start-arm-benchmarks/img_7.png)

Arm64 delivered **7-38% cost savings** for CPU-intensive workloads across all runtimes.

## Memory-Intensive Workload Results

| Runtime     | arm64 @10240MB | x86 @10240MB | arm64 Advantage |
| ----------- | -------------- | ------------ | --------------- |
| **Rust**    | 706ms          | 811ms        | +13%            |
| Node.js 20  | 1,900ms        | 2,623ms      | +28%            |
| Node.js 22  | 1,894ms        | 2,597ms      | +27%            |
| Python 3.11 | 9,178ms        | 12,717ms     | +28%            |

Arm64's advantage grows with memory allocation. At maximum 10GB Lambda configuration, arm64 was **27-28% faster** than x86 for Node.js workloads.

## Light Workload Results

For I/O-bound workloads, runtime differences largely disappear. All runtimes completed the light workload in 15-80ms at 512MB and above. The main takeaway: **optimize for cost, not raw performance.**

## Cold Start Analysis

| Runtime     | arm64 Init (avg) | x86 Init (avg) | arm64 Advantage |
| ----------- | ---------------- | -------------- | --------------- |
| **Rust**    | **16ms**         | 21ms           | +24%            |
| Python 3.11 | 79ms             | 94ms           | +16%            |
| Python 3.12 | 89ms             | 107ms          | +17%            |
| Python 3.13 | 100ms            | 122ms          | +18%            |
| Python 3.14 | 124ms            | 143ms          | +13%            |
| Node.js 20  | 134ms            | 155ms          | +13%            |
| Node.js 22  | 129ms            | 150ms          | +14%            |

**Rust cold starts are 5-8x faster than interpreted runtimes.** At 16ms on arm64, Rust initialization is nearly imperceptible.

Arm64 consistently showed **13-24% faster cold start initialization** across all runtimes.

Node.js 22 cold start on arm64: **129ms average** vs 155ms on x86.

### Cost Efficiency

![Scatter plot of cost vs performance for CPU-intensive workloads. arm64 points cluster lower-left (faster and cheaper). Rust arm64 closest to origin.](images/lambda-cold-start-arm-benchmarks/img_8.png)

- arm64 is **20% cheaper** per GB-second than x86
- arm64 performance matches or exceeds x86 in most cases
- Combined effect: **25-40% lower cost per invocation**

## Conclusions

**The verdict is clear: arm64 should be your default targeted CPU architecture for Lambda.** After multiple benchmark runs analyzing 183,750 Lambda invocations across 294 configurations, data consistently points in one direction.

### Why ARM64 Wins

| Benefit         | Impact                               |
| --------------- | ------------------------------------ |
| **Performance** | Equal or better in 90%+ of scenarios |
| **Cold starts** | 13-24% faster initialization         |
| **Cost**        | 25-40% lower per invocation          |

### Runtime Selection Guide

| Your Priority       | Best Choice                     |
| ------------------- | ------------------------------- |
| Maximum performance | Rust on arm64                   |
| Minimal cold starts | Rust on arm64 (16ms init)       |
| Python workloads    | Python 3.11 on arm64            |
| Node.js workloads   | Node.js 22 on arm64             |
| I/O-bound workloads | Any runtime — optimize for cost |

## Rust on ARM December Update

Enabling the `asm` feature on the `sha2` crate produced a **4-5x improvement for Rust on arm64** in CPU-intensive tests:

![Bar chart comparing Rust CPU-intensive performance before and after enabling ARM assembly-optimized SHA-256. arm64 drops from ~163ms to ~35ms at 2048MB; x86 unchanged at ~147ms.](images/lambda-cold-start-arm-benchmarks/img_9.png)

The drama of this improvement highlights that architecture performance depends heavily on whether dependencies leverage platform-specific optimizations (NEON on arm, SSE/AVX on x86).
