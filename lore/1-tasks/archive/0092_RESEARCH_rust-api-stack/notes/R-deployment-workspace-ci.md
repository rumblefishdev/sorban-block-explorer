---
type: research
status: mature
spawned_from: 0092
tags: [cargo-workspace, lambda-deployment, ci-cd, nx]
---

# Lambda Deployment, Cargo Workspace, CI (Q15-Q16)

## 1. Cargo Workspace Layout

Root `Cargo.toml` with `crates/` members. Coexists with Nx TypeScript monorepo.

**Updated 2026-03-31:** Revised after reviewing develop branch. Task 0024 created `apps/indexer/crates/xdr-parser/` (standalone crate inside Nx app). This needs to be migrated to the workspace. Also `apps/web/` moves to `web/` (only frontend, no reason for `apps/` wrapper when backends are in `crates/`).

```
soroban-block-explorer/
├── Cargo.toml              # Workspace root
├── Cargo.lock
├── crates/
│   ├── api/                # Binary: axum REST API Lambda
│   ├── indexer/            # Binary: Ledger Processor Lambda
│   ├── xdr-parser/         # Library: MIGRATE from apps/indexer/crates/xdr-parser/
│   ├── db/                 # Library: sqlx pool, queries, migrations
│   └── domain/             # Library: shared types, errors, config
├── web/                    # React frontend (move from apps/web/)
├── infra/                  # CDK stacks (flatten from infra/aws-cdk/)
├── libs/                   # TS libs for frontend (domain, shared, ui, api-types)
├── nx.json                 # Nx config (TS only)
└── package.json            # npm workspace root
```

**Removed after migration:** `apps/` directory entirely (api, indexer, workers — all replaced by `crates/`).

Root `Cargo.toml`:

```toml
[workspace]
resolver = "2"
members = [
    "crates/api",
    "crates/indexer",
    "crates/xdr-parser",
    "crates/db",
    "crates/domain",
]

[workspace.dependencies]
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sqlx = { version = "0.8", features = ["runtime-tokio-rustls", "postgres", "json", "chrono", "migrate"] }
axum = "0.8"
lambda_http = "1"
utoipa = { version = "5", features = ["axum_extras"] }
tracing = "0.1"
stellar-xdr = { version = "26", default-features = true, features = ["curr"] }
```

Note: `stellar-xdr` version bumped to 26 (matching xdr-parser on develop).

### Why 5 crates

| Crate        | Type    | Purpose                                                  | Key deps                                 |
| ------------ | ------- | -------------------------------------------------------- | ---------------------------------------- |
| `api`        | binary  | axum REST API Lambda handler                             | axum, lambda_http, utoipa, db            |
| `indexer`    | binary  | XDR ingestion, DB writes                                 | xdr-parser, db                           |
| `xdr-parser` | library | LedgerCloseMeta deserialization, ScVal conversion        | stellar-xdr, serde_json, sha2            |
| `domain`     | library | Shared types (Ledger, Transaction, etc.), errors, config | serde, chrono, thiserror (zero async/IO) |
| `db`         | library | PgPool setup, query functions, migrations                | sqlx, domain                             |

Dependency graph:

```
api → db → domain
indexer → xdr-parser → (stellar-xdr)
indexer → db → domain
xdr-parser → domain (for shared output types)
```

`domain` has zero async/IO deps → fast compile, pure data types. `db` handles sqlx mapping (FromRow), keeping domain types ORM-agnostic. `xdr-parser` is the existing crate from task 0024 (currently at `apps/indexer/crates/xdr-parser/`, needs migration to `crates/xdr-parser/`).

### Nx + Cargo Coexistence

No conflicts. Nx scans `apps/`, `libs/` via package.json. Cargo scans `rust/crates/` via workspace members. `target/` already in `.gitignore`/`.prettierignore`.

Wrap cargo commands as Nx targets via `nx:run-commands`:

```jsonc
// rust/project.json
{
  "name": "rust",
  "targets": {
    "build": { "command": "cargo lambda build --release --arm64" },
    "build-api": { "command": "cargo lambda build --release --arm64 -p api" },
    "test": { "command": "cargo test --workspace" },
    "lint": { "command": "cargo clippy --workspace -- -D warnings" },
    "fmt-check": { "command": "cargo fmt --all -- --check" }
  }
}
```

**Skip `@nxrs/cargo`** — stale (2+ years, ~675 weekly downloads), no Nx 22.x compat, no cargo-lambda awareness.

## 2. cargo-lambda in Workspace

Works out of the box with `-p <name>`:

```bash
cargo lambda build --release --arm64           # all binaries
cargo lambda build --release --arm64 -p api    # just API
cargo lambda build --release --arm64 -p indexer # just indexer
```

Output:

```
target/lambda/
├── api/bootstrap          # ARM64 API binary
└── indexer/bootstrap      # ARM64 Indexer binary
```

Local dev: `cargo lambda watch -p api` — hot recompile on port 9000.

## 3. CDK Integration

`cargo-lambda-cdk` `RustFunction` construct:

```typescript
import { RustFunction } from 'cargo-lambda-cdk';

const apiFn = new RustFunction(this, 'ApiFunction', {
  manifestPath: path.join(__dirname, '../../..'), // repo root
  binaryName: 'api',
  bundling: { cargoLambdaFlags: ['--arm64'] },
  architecture: Architecture.ARM_64,
  memorySize: 256,
});
```

`manifestPath` → workspace root (where root `Cargo.toml` is). `binaryName` → selects package. Construct runs `cargo lambda build` internally.

## 4. CI/CD Pipeline

### Job structure (parallel)

```yaml
jobs:
  node: # existing — npm ci → nx format:check → nx lint/build/typecheck
  rust: # new — cargo check → clippy → test → lambda build
  deploy: # depends on both
    needs: [node, rust]
```

### Rust job

```yaml
rust:
  runs-on: ubuntu-latest
  env:
    SQLX_OFFLINE: 'true'
  steps:
    - uses: actions/checkout@v4
    - uses: dtolnay/rust-toolchain@stable
      with: { components: clippy }
    - uses: Swatinem/rust-cache@v2
    - run: pip3 install cargo-lambda
    - run: cargo check --workspace
    - run: cargo clippy --workspace -- -D warnings
    - run: cargo test --workspace
    - run: cargo lambda build --release --arm64
    - uses: actions/upload-artifact@v4
      with:
        name: lambda-binaries
        path: target/lambda/
```

### Key decisions

- **Toolchain**: `dtolnay/rust-toolchain@stable` (standard, maintained)
- **Caching**: `Swatinem/rust-cache@v2` (caches registry + target, auto key rotation)
- **cargo-lambda install**: `pip3 install cargo-lambda` (fastest, ~5s)
- **ARM64 cross-compile**: `cargo lambda build --arm64` uses Zig linker, works on ubuntu-latest, no Docker/QEMU
- **sqlx offline**: `SQLX_OFFLINE=true` + committed `.sqlx/` directory
- **Artifact**: upload `target/lambda/` → download in deploy job

### Build times

| Step                                 | Cold         | Cached      |
| ------------------------------------ | ------------ | ----------- |
| cargo check                          | 2-3 min      | 15-30s      |
| cargo clippy                         | 30s          | 15s         |
| cargo test                           | 30s-1 min    | 15-30s      |
| cargo lambda build --release --arm64 | 3-5 min      | 30s-1 min   |
| **Total Rust job**                   | **7-10 min** | **2-3 min** |

Rust job runs fully parallel with Node.js job — adds zero time to TS CI path.

## 5. Binary Size (Q15)

Measured locally (macOS native, api-stack-test with axum+sqlx+utoipa+lambda_http):

- Unstripped: 6.8 MB
- Stripped: 5.6 MB
- Gzipped: 2.6 MB

ARM64 cross-compile + `opt-level="z"` + LTO will produce smaller binaries. Lambda ZIP limit: 50 MB. Well within limits.

Release profile for smallest binary:

```toml
[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
strip = true
```

## Sources

- cargo-lambda build docs: https://www.cargo-lambda.info/commands/build.html
- cargo-lambda-cdk: https://github.com/cargo-lambda/cargo-lambda-cdk
- dtolnay/rust-toolchain: https://github.com/dtolnay/rust-toolchain
- Swatinem/rust-cache: https://github.com/Swatinem/rust-cache
- Cargo workspaces: https://doc.rust-lang.org/cargo/reference/workspaces.html
