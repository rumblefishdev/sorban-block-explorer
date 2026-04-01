---
fetched: 2026-03-31
source: https://maxday.github.io/lambda-perf/ (data from 2026-03-30)
methodology: https://github.com/maxday/lambda-perf
---

# Lambda Cold Start Benchmarks — Rust on provided.al2023

## Methodology

- 10 cold start samples per runtime per day, freshly deployed each time
- Captures AWS Lambda REPORT log line (init duration, memory used, duration)
- Reports averages, not percentiles (10 samples too few for P50/P99)
- Region: us-east-1
- Updated daily

## Rust ARM64 ZIP Deploy Results (2026-03-30)

### 512 MB Memory

| Sample | Init Duration (ms) |
| ------ | ------------------ |
| 1      | 12.72              |
| 2      | 18.82              |
| 3      | 15.01              |
| 4      | 12.97              |
| 5      | 13.12              |
| 6      | 12.83              |
| 7      | 14.55              |
| 8      | 12.19              |
| 9      | 14.10              |
| 10     | 12.90              |

**Average: 13.921 ms** | **Min: 12.19 ms** | **Max: 18.82 ms** | **Memory used: 15 MB**

### All Memory Sizes (ARM64, ZIP)

| Memory  | Avg Cold Start | Range            |
| ------- | -------------- | ---------------- |
| 128 MB  | 13.032 ms      | 11.64 - 14.79 ms |
| 256 MB  | 13.775 ms      | 11.95 - 16.21 ms |
| 512 MB  | 13.921 ms      | 12.19 - 18.82 ms |
| 1024 MB | 13.384 ms      | 11.61 - 15.19 ms |

## Corrections to Initial Research

| Claim          | Was     | Verified                                                   |
| -------------- | ------- | ---------------------------------------------------------- |
| Cold start P50 | 12-22ms | **Avg ~14ms** (range 12-19ms)                              |
| Cold start P99 | 40-60ms | **Max 18.82ms** for ZIP (40-60ms is container images only) |
| ARM64 init     | 19-23ms | **13-14ms avg** (19-23ms is x86_64 range)                  |

## Container Image Deploy (for reference)

Container image deploys show 40-60ms cold starts — ~3-4x slower than ZIP. Our deployment uses ZIP (cargo-lambda build → zip → deploy).
