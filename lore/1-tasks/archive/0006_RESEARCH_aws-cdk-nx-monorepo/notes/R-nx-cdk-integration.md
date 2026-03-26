---
title: 'Nx workspace integration with CDK: build outputs, targets, and Rust Lambda bundling'
type: research
status: developing
spawned_from: null
spawns: []
tags: [nx, cdk, lambda, rust, asset-bundling]
links:
  - https://docs.aws.amazon.com/cdk/v2/guide/assets.html
  - https://github.com/cargo-lambda/cargo-lambda-cdk
  - https://github.com/castleadmin/nx-plugins/tree/main/nx-serverless-cdk/plugin
  - https://docs.aws.amazon.com/lambda/latest/dg/rust-package.html
history:
  - date: 2026-03-26
    status: developing
    who: stkrolikiewicz
    note: 'Initial research on Nx-CDK asset flow and Rust Lambda deployment'
---

# Nx workspace integration with CDK: build outputs, targets, and Rust Lambda bundling

## CDK as an Nx Project

The CDK project already exists at `infra/aws-cdk` with Nx tags `type:infra, scope:infra`. It needs:

1. **Build target** — `tsc` compilation of CDK TypeScript code
2. **Synth target** — `cdk synth` to generate CloudFormation templates
3. **Deploy target** — `cdk deploy` for actual deployment
4. **Diff target** — `cdk diff` for PR previews

### Nx project.json targets

```json
{
  "targets": {
    "build": {
      "executor": "@nx/js:tsc",
      "options": {
        "outputPath": "dist/infra/aws-cdk",
        "tsConfig": "infra/aws-cdk/tsconfig.lib.json",
        "main": "infra/aws-cdk/src/index.ts"
      }
    },
    "synth": {
      "executor": "nx:run-commands",
      "dependsOn": ["build", "^build"],
      "options": {
        "command": "npx cdk synth",
        "cwd": "infra/aws-cdk"
      }
    },
    "deploy": {
      "executor": "nx:run-commands",
      "dependsOn": ["synth"],
      "options": {
        "command": "npx cdk deploy --all --require-approval never",
        "cwd": "infra/aws-cdk"
      }
    },
    "diff": {
      "executor": "nx:run-commands",
      "dependsOn": ["synth"],
      "options": {
        "command": "npx cdk diff",
        "cwd": "infra/aws-cdk"
      }
    }
  }
}
```

**Key:** `dependsOn: ["^build"]` ensures all app dependencies (api, web, indexer, workers) are built before CDK synth/deploy.

## Nx → CDK Asset Flow

### Node.js Lambdas (API, Event Interpreter)

CDK references pre-built Nx output directories:

```typescript
import * as lambda from 'aws-cdk-lib/aws-lambda';
import * as path from 'path';

// Reference Nx build output — NOT rebuilding during cdk deploy
const apiHandler = new lambda.Function(this, 'ApiHandler', {
  runtime: lambda.Runtime.NODEJS_22_X,
  architecture: lambda.Architecture.ARM_64,
  handler: 'index.handler',
  code: lambda.Code.fromAsset(
    path.join(__dirname, '../../../../apps/api/dist')
  ),
  // ... other config
});
```

**Important:** The path `../../../../apps/api/dist` is relative from `infra/aws-cdk/lib/stacks/`. This works because:

1. `nx build api` produces `apps/api/dist/`
2. CDK `synth` depends on `^build` — Nx builds apps first
3. `Code.fromAsset()` bundles the directory at synth time

### Rust Lambda (Ledger Processor)

Per ADR-0002, the Ledger Processor is a Rust static binary. Two options:

#### Option A: `cargo-lambda-cdk`

Community CDK construct ([cargo-lambda/cargo-lambda-cdk](https://github.com/cargo-lambda/cargo-lambda-cdk)) that wraps `cargo-lambda`:

```typescript
import { RustFunction } from 'cargo-lambda-cdk';

const processor = new RustFunction(this, 'LedgerProcessor', {
  manifestPath: path.join(__dirname, '../../../../apps/indexer-rs/Cargo.toml'),
  // Default runtime: provided.al2023
  // Supports local bundling (cargo-lambda installed) or Docker fallback
});
```

**Pros:** Handles cross-compilation, bundling, packaging. Supports Docker fallback if `cargo-lambda` not installed.
**Cons:** Builds during `cdk synth` (not Nx-managed). Adds build time to synth.

Note: `@cdklabs/aws-lambda-rust` (v0.0.10) also exists but is less mature than `cargo-lambda-cdk`.

#### Option B: Pre-built binary with `Code.fromAsset()`

Build Rust binary in CI before `cdk synth`, reference the artifact:

```typescript
const processor = new lambda.Function(this, 'LedgerProcessor', {
  runtime: lambda.Runtime.PROVIDED_AL2023,
  architecture: lambda.Architecture.ARM_64,
  handler: 'bootstrap', // Rust binary name
  code: lambda.Code.fromAsset(
    path.join(
      __dirname,
      '../../../../apps/indexer-rs/target/lambda/explore-xdr'
    )
  ),
});
```

**Pros:** Full control over build. Nx can manage the Rust build target. No dependency on `@cdklabs/aws-lambda-rust`.
**Cons:** Must set up cross-compilation for ARM64 Lambda manually. More CI config.

#### Recommendation: Option B (pre-built binary)

Reasons:

1. `@cdklabs/aws-lambda-rust` is v0.0.10 — too early for production
2. Nx should manage ALL builds, including Rust — consistent build graph
3. Pre-built binary is simpler to debug and cache in CI
4. CI can use `cargo lambda build --release --arm64` in a Docker container

### Nx Rust Build Target

Add a build target for the Rust Lambda in `apps/indexer-rs/project.json`:

```json
{
  "targets": {
    "build": {
      "executor": "nx:run-commands",
      "options": {
        "command": "cargo lambda build --release --arm64",
        "cwd": "apps/indexer-rs"
      },
      "outputs": ["{projectRoot}/target/lambda"]
    }
  }
}
```

CDK `synth` depends on this via Nx `dependsOn`.

### Frontend (React SPA)

```typescript
const frontendBucket = new s3.Bucket(this, 'FrontendBucket', {
  /* ... */
});

new s3deploy.BucketDeployment(this, 'DeployFrontend', {
  sources: [
    s3deploy.Source.asset(path.join(__dirname, '../../../../apps/web/dist')),
  ],
  destinationBucket: frontendBucket,
});
```

## Summary: Build Flow

```
nx build api          → apps/api/dist/          → CDK Code.fromAsset()
nx build workers      → apps/workers/dist/      → CDK Code.fromAsset()
nx build web          → apps/web/dist/          → CDK BucketDeployment
nx build indexer-rs   → apps/indexer-rs/target/  → CDK Code.fromAsset() (Rust binary)
nx build aws-cdk      → infra/aws-cdk/dist/     → CDK TypeScript compiled
nx synth aws-cdk      → cdk.out/                → CloudFormation templates
nx deploy aws-cdk     → CloudFormation deploy   → AWS resources
```

All builds are Nx-managed. CDK only references pre-built artifacts.

## Rust Cross-Compilation for Lambda

`cargo lambda build --release --arm64` targets `aarch64-unknown-linux-musl`. On macOS (ARM or x86), this requires cross-compilation:

**Option 1: Docker (recommended for CI)**

```bash
cargo lambda build --release --arm64 --lambda-dir target/lambda
# cargo-lambda automatically uses Docker for cross-compilation when needed
```

`cargo-lambda` detects the host architecture and uses a Docker container with the correct Linux cross-compiler. No manual toolchain setup needed.

**Option 2: Zig cross-compiler (local dev)**

```bash
# Install zig
brew install zig

# cargo-lambda uses zig as cross-linker automatically
cargo lambda build --release --arm64
```

`cargo-lambda` supports Zig as a lighter alternative to Docker for cross-compilation.

**CI setup (GitHub Actions):**

```yaml
- name: Install Rust toolchain
  uses: dtolnay/rust-toolchain@stable
  with:
    targets: aarch64-unknown-linux-gnu

- name: Install cargo-lambda
  run: pip3 install cargo-lambda

- name: Build Rust Lambda
  run: cargo lambda build --release --arm64 --manifest-path apps/indexer-rs/Cargo.toml
```

There is no official `cargo-lambda` GitHub Action. Install via `pip3` (fastest) or `cargo install cargo-lambda` (slower, compiles from source). `dtolnay/rust-toolchain` is the standard Rust toolchain action.

## CDK Context File (`cdk.context.json`)

**Must be committed to git.** CDK writes lookup results (VPC IDs, AZ info, AMI IDs) to `cdk.context.json` during synthesis. If not committed:

1. **Non-deterministic synthesis** — different runs may resolve different VPC/AZ values
2. **CI failures** — CI environment can't perform runtime lookups without AWS credentials during build
3. **Drift** — team members synthesize different templates from the same code

```gitignore
# DO NOT ignore cdk.context.json — it ensures deterministic synthesis
# cdk.context.json  ← intentionally NOT ignored
```

AWS CDK best practices explicitly recommend committing this file. See [source](../sources/aws-cdk-best-practices.md).
