---
url: 'https://www.gomomento.com/blog/how-we-turned-up-the-heat-on-node-js-lambda-cold-starts/'
title: 'How we turned up the heat on Node.js Lambda cold starts'
author: 'Matt Straathof'
date: '2023-11-16'
fetched_date: 2026-03-26
task_id: '0004'
overwritten: false
image_count: 0
---

# How We Turned Up the Heat on Node.js Lambda Cold Starts

**Practical esbuild optimization that reduced cold starts by 40-90%**

## Overview

A customer's initial Node.js Lambda function using Momento experienced cold start times exceeding **1 second** for a simple 25-line codebase making a single gRPC call. This prompted an investigation into optimization techniques that ultimately achieved a **90% reduction** in cold start time.

## Initial Setup

- Simple 25-line codebase, single dependency (`@gomomento/sdk`)
- Built with Webpack, deployed via Serverless Framework
- Original bundle size: **1.5MB**
- Cold start times: **~1000ms**
- Configuration: 768MB memory

## Memory Configuration: Minimal Impact

Testing memory at 256MB, 512MB, and 1024MB showed **no meaningful impact** on cold start performance. Memory allocation improves warm start execution (CPU scaling) but does not speed up Lambda container initialization or module loading.

## The esbuild Transformation

Installing the `serverless-esbuild` plugin transformed results:

```bash
npm i --save-dev serverless-esbuild
```

`serverless.yml` additions:

```yaml
plugins:
  - serverless-esbuild

custom:
  esbuild:
    bundle: true
    minify: true
    packager: 'npm'
    target: 'node18'
    watch:
      pattern: ['src/**/*.js']
```

**Customer results:**

- Bundle size: 1.5MB → **~260KB** (83% reduction)
- Cold start time: ~1000ms → **~100ms** (90% reduction)

## Momento's Internal Optimization

Starting point for Momento's production Lambda:

- Bundle size: **2.8MB**
- Architecture: x86
- Memory: 256MB
- Baseline cold start: **~1000ms**

### esbuild Bundling Result

Bundle size after esbuild: 2.8MB → **677KB**

| Memory | Cold Start (677KB) |
| ------ | ------------------ |
| 256MB  | ~600ms             |
| 512MB  | ~550ms             |
| 768MB  | ~500ms             |

### Minification Impact

Adding `minify: true` resulted in the bundle only reducing to **2.7MB** — minification alone provides limited benefits when the bulk of size is third-party dependencies.

### Source Map Externalization

Changing from `sourcemap: 'inline'` to `sourcemap: 'external'` reduced bundle to **590KB**:

```
dist
├── handler.js        ← deployed
├── handler.js.map    ← kept locally for debugging
└── handler.zip
```

### Name Minification

Setting `keepNames: false` reduces bundle to **566KB**:

| Memory | Cold Start (566KB) |
| ------ | ------------------ |
| 256MB  | ~650ms             |
| 512MB  | ~600ms             |
| 768MB  | ~550ms             |

**Overall result: 40% cold start reduction** from the 2.8MB original.

### AWS SDK Externalization: A Cautionary Tale

Attempting to externalize the AWS SDK (`external: ['@aws-sdk/*']`) reduced bundle to **375KB** but paradoxically **increased cold starts back to ~1000ms** — the dynamic linking overhead at cold start time outweighed the smaller bundle size.

This approach was abandoned because:

- It prevented local execution (SDK not in bundle)
- Risked AWS SDK version incompatibility (relying on Lambda's built-in version)

> Lesson: Externalizing the AWS SDK is a common optimization suggestion that **backfires** for many workloads. Always benchmark, don't assume.

## Final esbuild Configuration

```javascript
import * as fs from 'fs';
import * as path from 'path';
import { build } from 'esbuild';

const functionsDir = 'src';
const outDir = 'dist';
const entryPoints = fs
  .readdirSync(path.join(__dirname, functionsDir))
  .filter((entry) => entry !== 'common')
  .map((entry) => `${functionsDir}/${entry}/handler.ts`);

build({
  entryPoints,
  bundle: true,
  outdir: path.join(__dirname, outDir),
  outbase: functionsDir,
  platform: 'node',
  sourcemap: 'external',
  write: true,
  tsconfig: './tsconfig.json',
  minify: true,
  keepNames: false,
}).catch(() => process.exit(1));
```

## Architecture Note

Testing revealed **ARM64 underperformed x86 by approximately 15%** within Lambda environments for this gRPC-heavy workload. This contradicts positive ARM benchmarks on EC2 instances. Workload characteristics (gRPC, I/O patterns) matter — generic architecture benchmarks may not apply to all use cases.

## Memory Optimization Results

| Memory | Cold Start | Warm Start |
| ------ | ---------- | ---------- |
| 256MB  | ~650ms     | ~50ms      |
| 512MB  | ~600ms     | ~45ms      |
| 768MB  | ~550ms     | ~40ms      |
| 1024MB | ~500ms     | ~35ms      |

Optimal performance-to-cost ratio falls between **512MB and 768MB**.

## Key Takeaways

1. **Bundle with esbuild** — biggest single improvement; can reduce bundle 75-90%
2. **Use external source maps** — saves bundle size without losing debuggability
3. **Enable `keepNames: false`** — safe additional size reduction
4. **Don't externalize AWS SDK** — counterintuitively increases cold starts due to dynamic linking
5. **Memory alone does not help cold starts** — only helps warm execution speed
6. **Benchmark architecture choices** — ARM64 advantages vary by workload

---

# Addendum: esbuild with AWS SAM (from chrisarmstrong.dev)

Source: Package your NodeJS Lambda functions individually with esbuild for faster cold-start times

## Why Package with a Bundler?

- **Improved cold start time**: Smaller deployment packages = less time copying the package to the Lambda container
- **TypeScript support**: Native TS transpilation without Babel
- **Tree-shaking**: Only include code that's actually imported; reduces bundle from 50+ MB node_modules to < 1MB

## esbuild vs webpack

|          | esbuild                                     | webpack                   |
| -------- | ------------------------------------------- | ------------------------- |
| Language | Go                                          | JavaScript                |
| Speed    | <30 seconds for large projects              | 5+ minutes                |
| Maturity | Less mature, deliberate reduced feature set | Mature, very flexible     |
| Memory   | Efficient                                   | Can OOM on large projects |

esbuild reduced bundling times from **over 5 minutes to less than 30 seconds** for a large Lambda project.

## AWS SAM esbuild Configuration

```javascript
const fs = require('fs');
const path = require('path');
const esbuild = require('esbuild');

const functionsDir = 'src';
const outDir = 'dist';
const entryPoints = fs
  .readdirSync(path.join(__dirname, functionsDir))
  .map((entry) => `${functionsDir}/${entry}/index.ts`);

esbuild.build({
  entryPoints,
  bundle: true,
  outdir: path.join(__dirname, outDir),
  outbase: functionsDir,
  platform: 'node',
  sourcemap: 'inline',
});
```

Source layout for individual function packaging:

```
src/say_hello/index.ts
src/send_email/index.ts
```

AWS SAM template using individually bundled functions:

```yaml
Resources:
  HelloFunction:
    CodeUri: dist/say_hello
    Handler: index.handler
  SendFunction:
    CodeUri: dist/send_email
    Handler: index.handler
```

Build and deploy workflow:

```bash
node esbuild.js    # bundle first
sam package
sam deploy
```

> **Note (2024):** AWS SAM now supports esbuild natively — the manual script approach is no longer required. See the [SAM TypeScript documentation](https://docs.aws.amazon.com/serverless-application-model/latest/developerguide/serverless-sam-cli-using-build-typescript.html).

## Disadvantages of Bundling

- **Increased complexity**: More steps before `sam local` invocation
- **Poor stack trace support**: Bundled code makes stack traces harder to read
- **Module incompatibility**: Some modules assume existence of `package.json`/`node_modules` or perform auto-instrumentation (e.g., OpenTelemetry auto-instrumentation)
