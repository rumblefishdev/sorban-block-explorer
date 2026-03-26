---
prefix: R
title: 'Cold Start Benchmarks on ARM/Graviton2'
status: mature
spawned_from: '0004'
---

# R: Cold Start Benchmarks on ARM/Graviton2

## Node.js Runtime Cold Start (bare)

Based on a comprehensive 183,750-invocation benchmark (Nov 2025):

| Runtime    | arm64 Init (avg) | x86 Init (avg) | arm64 Advantage |
| ---------- | ---------------- | -------------- | --------------- |
| Node.js 22 | **129ms**        | 150ms          | +14%            |
| Node.js 20 | 134ms            | 155ms          | +13%            |

> Source: [lambda-cold-start-arm-benchmarks.md](../sources/lambda-cold-start-arm-benchmarks.md), "Cold Start Analysis" table

**Key finding:** ARM is 13-14% faster on cold starts for Node.js, with Node.js 22 slightly outperforming Node.js 20.

## NestJS Application Cold Start

NestJS adds significant overhead on top of bare runtime init:

| Scenario                                        | Init Duration | Source                                                                                          |
| ----------------------------------------------- | ------------- | ----------------------------------------------------------------------------------------------- |
| NestJS mono-lambda (production, unbundled)      | **1.0-1.1s**  | [nestjs-lambda-cold-starts-mono-lambda.md](../sources/nestjs-lambda-cold-starts-mono-lambda.md) |
| NestJS with Express (webpack bundled, local)    | **81.5ms**    | [nestjs-serverless-docs.md](../sources/nestjs-serverless-docs.md)                               |
| NestJS 10-resource app (webpack bundled, local) | **129.8ms**   | [nestjs-serverless-docs.md](../sources/nestjs-serverless-docs.md)                               |

**Important notes:**

- The 81.5ms and 129.8ms figures are LOCAL benchmarks on a MacBook, not Lambda. They demonstrate webpack bundling impact, not real Lambda init duration.
- The 1.0-1.1s figure is from a real Lambda deployment but without esbuild optimization.
- With esbuild bundling, Lambda cold starts can be reduced by 40-90% compared to unbundled ([lambda-bundle-size-esbuild-optimization.md](../sources/lambda-bundle-size-esbuild-optimization.md)).

## Estimated Cold Start for Our 9-Module App

Extrapolating from the data:

- **Unbundled on Lambda**: ~1.0-1.5s (based on the mono-lambda article's 1.0-1.1s for a similar-scale app)
- **With esbuild bundling on ARM**: ~400-600ms estimated (applying 40-60% reduction from bundling)
- **With esbuild + memory optimization (1024MB+)**: ~300-500ms estimated

> The NestJS 10-resource benchmark shows 129.8ms bundled locally. Lambda adds overhead for container init, code download, and runtime bootstrap (~129ms for Node.js 22 arm64), suggesting a total of ~300-500ms for a well-optimized bundle.

## Cold Start Frequency

At moderate traffic (10 req/s), cold starts impact **less than 0.5%** of requests:

- 4 cold starts out of ~1000 requests in a 2-minute test
- Warm response time: ~70ms average

> Source: [nestjs-lambda-cold-starts-mono-lambda.md](../sources/nestjs-lambda-cold-starts-mono-lambda.md), "Key Metrics" section

## Optimization Levers

### 1. esbuild Bundling (highest impact)

- Reduces bundle size by 75-90%
- Cold start reduction: 40-90%
- AWS officially recommends esbuild for Node.js 18+ runtimes

> Source: [lambda-bundle-size-esbuild-optimization.md](../sources/lambda-bundle-size-esbuild-optimization.md)

### 2. Memory Allocation

AWS allocates CPU proportionally to memory. **1 full vCPU requires 1,769 MB**. For single-threaded NestJS bootstrap, increasing memory from 128MB to 1024MB+ significantly reduces cold start.

> Source: [lambda-cold-start-arm-benchmarks.md](../sources/lambda-cold-start-arm-benchmarks.md), methodology section

### 3. Node.js 22 + ARM64

Node.js 22 arm64 is the fastest Node.js Lambda configuration:

- 8-11% faster execution than Node.js 20
- 14% faster cold start on arm64 vs x86
- 25-40% lower cost per invocation

> Source: [lambda-cold-start-arm-benchmarks.md](../sources/lambda-cold-start-arm-benchmarks.md), conclusions

## Cost Impact of ARM

| Benefit     | Impact                            |
| ----------- | --------------------------------- |
| Performance | Equal or better in 90%+ scenarios |
| Cold starts | 13-24% faster initialization      |
| Cost        | **25-40% lower** per invocation   |

> Source: [lambda-cold-start-arm-benchmarks.md](../sources/lambda-cold-start-arm-benchmarks.md), "Why ARM64 Wins" table
