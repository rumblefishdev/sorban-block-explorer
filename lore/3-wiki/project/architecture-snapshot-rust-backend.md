# Architecture Snapshot: Rust Backend

> Living doc — current state of the Rust backend. For historical versions use
> `git log --follow lore/3-wiki/project/architecture-snapshot-rust-backend.md`.
> Detailed architecture lives under [`docs/architecture/**`](../../../docs/architecture)
> (evergreen per [ADR 0032](../../2-adrs/0032_docs-architecture-evergreen-maintenance.md)).

This file is a narrative cross-cut across the evergreen docs — useful for
onboarding a stateless session in one read. It intentionally does not restate
DDL, endpoint contracts, or deployment details; it points to where those live.

## Stack

| Layer                | Technology                                                         | Version     |
| -------------------- | ------------------------------------------------------------------ | ----------- |
| Web framework        | axum                                                               | 0.8         |
| Lambda runtime       | lambda_http / lambda_runtime                                       | 1           |
| OpenAPI              | utoipa + utoipa-axum + utoipa-swagger-ui                           | 5 / 0.2 / 9 |
| Database             | sqlx (no ORM; compile-time-checked queries)                        | 0.8         |
| Middleware           | tower-http (cors, trace)                                           | 0.6         |
| XDR                  | stellar-xdr (`features = ["curr"]`)                                | 26          |
| Observability        | tracing + tracing-subscriber                                       | 0.1 / 0.3   |
| AWS SDK              | aws-config, aws-sdk-s3, aws-sdk-cloudwatch, aws-sdk-secretsmanager | 1           |
| Compression          | zstd                                                               | 0.13        |
| Build tool           | cargo-lambda                                                       | 1           |
| Rust edition         | 2024 (all crates)                                                  | —           |
| Postgres (local dev) | `postgres:16-alpine` via `docker-compose.yml`                      | 16          |
| Monorepo runner      | Nx                                                                 | 22.6.1      |

## Workspace Layout

```text
.
├── Cargo.toml            # Rust workspace root
├── nx.json               # Nx monorepo config
├── crates/               # 9 Rust crates (edition 2024)
├── web/                  # React 19 SPA (Vite 7, MUI 7, TanStack Query 5)
├── libs/ui/              # Shared React component library
├── infra/                # TypeScript CDK (stacks in infra/src/lib/stacks/)
├── docs/architecture/    # Evergreen architecture docs (per ADR 0032)
├── docs/audits/          # Point-in-time audit reports
├── .github/workflows/    # ci.yml, deploy-staging.yml, deploy-board.yml
├── docker-compose.yml    # Local Postgres 16
├── scripts/              # Dev tooling
├── tools/                # Dev tooling
└── lore/                 # Task / ADR / wiki knowledge base
```

## Crates

| Crate               | Role                                                                                                                                                                                                                                                              |
| ------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `api`               | REST API Lambda (axum + utoipa). Read path. Bootstrapping — see [API Bootstrap Status](#api-bootstrap-status).                                                                                                                                                    |
| `indexer`           | Ledger Processor Lambda (`lib` + `main`). S3-event-triggered by the galexie ledger bucket. Four parsing stages + one atomic DB transaction per ledger.                                                                                                            |
| `xdr-parser`        | `LedgerCloseMeta` → domain-typed output. DB/HTTP-free. Reused by `indexer`, `backfill-runner`, and API read-time fetch. Modules: ledger, transaction, operation, event, invocation, contract, envelope, memo, nft, scval, classification, ledger_entry_changes, … |
| `db`                | sqlx pool, Secrets Manager resolver, compile-time-embedded migration harness (`sqlx::migrate!("./migrations")`).                                                                                                                                                  |
| `db-migrate`        | Migration Lambda. CloudFormation custom resource: on Create/Update calls `db::migrate::run_migrations` via RDS Proxy; on Delete no-op.                                                                                                                            |
| `db-partition-mgmt` | Partition management Lambda. CFN custom resource + EventBridge scheduled. `lib` + `bin` split so logic is unit-testable.                                                                                                                                          |
| `domain`            | Shared types + `#[repr(i16)]` enums ([ADR 0031](../../2-adrs/0031_enum-columns-smallint-with-rust-enum.md)). Feature-gated `sqlx::Type` / `utoipa::ToSchema` so `xdr-parser` stays sqlx-free.                                                                     |
| `backfill-bench`    | Local benchmark: public S3 → local Postgres.                                                                                                                                                                                                                      |
| `backfill-runner`   | Production backfill: public S3 → Postgres (parse-and-persist).                                                                                                                                                                                                    |

## Data Model

**Authoritative schema reference:** [ADR 0037 — current schema snapshot](../../2-adrs/0037_current-schema-snapshot.md).
Do not restate DDL here.

Shape highlights:

- **Surrogate `BIGINT` ids.** `accounts.id`, `contracts.id` replace StrKey
  `VARCHAR(56)` FKs across hot tables
  ([ADR 0026](../../2-adrs/0026_accounts-surrogate-bigint-id.md),
  [ADR 0030](../../2-adrs/0030_contracts-surrogate-bigint-id.md)).
  ADR 0030 supersedes ADR 0027 as the schema baseline.
- **Binary hashes.** Hash-derived columns (transaction hashes, pool IDs,
  etc.) stored as `BYTEA(32)` rather than `VARCHAR(64)` hex
  ([ADR 0024](../../2-adrs/0024_hashes-bytea-binary-storage.md)).
- **`SMALLINT` enums.** Nine enum-like columns flipped from `VARCHAR` to
  `SMALLINT + CHECK`, backed by six Rust enums in `crates/domain/src/enums/`
  and `IMMUTABLE` SQL label helper functions
  ([ADR 0031](../../2-adrs/0031_enum-columns-smallint-with-rust-enum.md)).
- **`*_appearances` index pattern.** Per-node / per-event detail removed
  from the DB; the row collapses to a `(contract, transaction, ledger, amount)`
  index tuple. Detail is reconstructed at read time from XDR. Applies to:
  - `soroban_events_appearances` — [ADR 0033](../../2-adrs/0033_soroban-events-appearances-read-time-detail.md)
  - `soroban_invocations_appearances` — [ADR 0034](../../2-adrs/0034_soroban-invocations-appearances-read-time-detail.md) (retains `caller_id` payload)
  - `operations_appearances` — task 0163
- **Assets (not tokens).** Table renamed `tokens` → `assets`; label
  `asset_type = 'classic'` → `'classic_credit'`
  ([ADR 0036](../../2-adrs/0036_rename-tokens-to-assets.md)).
- **`account_balance_history` dropped**
  ([ADR 0035](../../2-adrs/0035_drop-account-balance-history.md)).
- **Migrations.** Base migrations `0001_extensions` … `0007_account_balances`
  (post-task-0140 big-bang implementing the ADR 0027 → 0030 lineage) live
  in `crates/db/migrations/`. Additive changes use reversible timestamped
  pairs `YYYYMMDDHHMMSS_name.{up,down}.sql`. Migrations are embedded at
  compile time in the `db` crate.

## Write Path

```text
Galexie (self-hosted)
  │ writes raw .xdr.zst
  ▼
Ledger ingestion S3 bucket ── PutObject ──► indexer Lambda
                                              │
                                   downloads + decompresses XDR
                                              │
                              ┌── Ledger + transaction extraction
                              ├── Operation extraction
                              ├── Events, invocations, contract interfaces
                              └── Ledger entry changes + derived state
                                              │
                              one atomic DB transaction per ledger
                                              │ via RDS Proxy
                                              ▼
                                       Postgres (RDS)
```

Parallel historical path:

```text
Public Stellar archive ──► backfill-runner ──► Postgres (via RDS Proxy)
  s3://aws-public-blockchain/      (task 0145, parse-and-persist)
  v1.1/stellar/ledgers/pubnet/
```

Notes:

- The ADR 0029 pivot removed the planned intermediate "parsed-artifact-producer"
  Lambda (task 0147 superseded by 0149). The existing indexer Lambda subscribes
  directly to galexie's ledger bucket via S3 `ObjectCreated` and both parses
  and persists in one invocation.
- Legacy pre-0149 write path removed (task 0148); shared parse logic now lives
  in `xdr-parser` + `indexer/handler/process.rs` + `indexer/handler/persist/`.
- RDS Proxy sits between every Lambda and Postgres (fix from task 0116
  concurrency-exhaustion incident).
- `indexer/handler/persist/classification_cache.rs` is a per-worker cache of
  SEP-0050 (NFT) vs SEP-0041 (fungible) WASM-signature classifications. Task
  0118 Phase 1 ships the pure classifier in `xdr-parser/src/classification.rs`;
  Phase 2 write-time filtering is scaffolded but task 0118 remains blocked.
- Indexer publishes `LastProcessedLedgerSequence` to CloudWatch.
- Partition creation / pruning handled by `db-partition-mgmt` on an EventBridge
  schedule — see [`partition-pruning-runbook.md`](../partition-pruning-runbook.md).

## Read Path

**Pivot** ([ADR 0029](../../2-adrs/0029_abandon-parsed-artifacts-read-time-xdr-fetch.md)):
no parsed-ledger S3 bucket on our side. Heavy fields (memos, signatures,
event topics/data, invocation tree nodes) are fetched on demand from the
public Stellar archive
(`s3://aws-public-blockchain/v1.1/stellar/ledgers/pubnet/`) and parsed
inline when the response is assembled.

Fetch infrastructure lives in `crates/api/src/stellar_archive/`:

| Module          | Role                                                         |
| --------------- | ------------------------------------------------------------ |
| `key.rs`        | Build public-archive S3 keys from ledger sequence            |
| `extractors.rs` | Pull heavy fields out of fetched XDR                         |
| `merge.rs`      | Merge DB index rows with XDR-extracted detail into responses |
| `dto.rs`        | Response shapes                                              |

Split: DB → cheap index + identity; XDR fetch → expensive detail. Per-endpoint
split follows the ADR 0033 / 0034 coverage-matrix shifts.

## API Bootstrap Status

The `api` crate is still wiring router / state / error infrastructure.
Currently present:

- `config`, `state` (`AppState`), `openapi` (doc + schemas)
- `/health` liveness probe
- `transactions/` (handlers, cursor, dto, queries) — E2 / E3
- `contracts/` (handlers, cursor, dto, queries, cache) — E10 / E11 / E13 / E14
  (task 0050). Detail + interface go to Postgres; invocations + events fan out
  to `stellar_archive::fetch_ledgers` and re-extract per-node detail through
  `xdr_parser::extract_invocations` / `extract_events`. Detail responses are
  cached for 45 s in `ContractMetadataCache` (per-Lambda warm container).
- `stellar_archive/` fetch helpers (ready for consumers)

Handlers still pending: E12 (operations summary read-path) and the rest of
the M2 endpoint surface (ledgers, accounts, assets, NFTs, pools, search).

Active tasks extending this crate:

- **0123** — XDR decoding service (advanced transaction view)
- **0160** — SAC asset identity extraction (indexer/xdr-parser bug; `assets`
  table currently empty after reindex)

## Infrastructure (CDK, TypeScript)

Stacks under `infra/src/lib/stacks/`:

| Stack                 | Role                                  |
| --------------------- | ------------------------------------- |
| `rds-stack`           | Postgres RDS + RDS Proxy              |
| `ingestion-stack`     | Galexie + indexer Lambda wiring       |
| `ledger-bucket-stack` | Raw ledger ingestion bucket (galexie) |
| `compute-stack`       | Shared Lambda compute resources       |
| `api-gateway-stack`   | API Gateway → `api` Lambda            |
| `delivery-stack`      | Frontend delivery (CloudFront / S3)   |
| `bastion-stack`       | VPC bastion host                      |
| `cloudwatch-stack`    | Alarms and dashboards                 |
| `cicd-stack`          | CI/CD pipelines                       |

`db-migrate` and `db-partition-mgmt` Lambdas are wired as CloudFormation
custom resources (migrations on deploy; partition management on deploy plus
scheduled EventBridge invocations).

## CI/CD

GitHub Actions workflows under `.github/workflows/`:

| Workflow             | Role                                                          |
| -------------------- | ------------------------------------------------------------- |
| `ci.yml`             | Per-PR build / lint / test                                    |
| `deploy-staging.yml` | Staging deploy pipeline                                       |
| `deploy-board.yml`   | Lore board regeneration + GH Pages deploy (on `develop` push) |

## Where to Read Next

| Topic                       | File                                                                                                                                                                                                 |
| --------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Full technical design       | [`docs/architecture/technical-design-general-overview.md`](../../../docs/architecture/technical-design-general-overview.md)                                                                          |
| Backend detail              | [`docs/architecture/backend/backend-overview.md`](../../../docs/architecture/backend/backend-overview.md)                                                                                            |
| Indexing pipeline           | [`docs/architecture/indexing-pipeline/indexing-pipeline-overview.md`](../../../docs/architecture/indexing-pipeline/indexing-pipeline-overview.md)                                                    |
| Schema                      | [`docs/architecture/database-schema/database-schema-overview.md`](../../../docs/architecture/database-schema/database-schema-overview.md) + [ADR 0037](../../2-adrs/0037_current-schema-snapshot.md) |
| XDR parsing                 | [`docs/architecture/xdr-parsing/xdr-parsing-overview.md`](../../../docs/architecture/xdr-parsing/xdr-parsing-overview.md)                                                                            |
| Infrastructure              | [`docs/architecture/infrastructure/infrastructure-overview.md`](../../../docs/architecture/infrastructure/infrastructure-overview.md)                                                                |
| Frontend                    | [`docs/architecture/frontend/frontend-overview.md`](../../../docs/architecture/frontend/frontend-overview.md) + [`frontend-stack.md`](./frontend-stack.md)                                           |
| Partition runbook           | [`../partition-pruning-runbook.md`](../partition-pruning-runbook.md)                                                                                                                                 |
| Ledger archive reference    | [`../stellar-pubnet-ledger-archive.md`](../stellar-pubnet-ledger-archive.md)                                                                                                                         |
| Pipeline audit (2026-04-10) | [`../../../docs/audits/2026-04-10-pipeline-data-audit.md`](../../../docs/audits/2026-04-10-pipeline-data-audit.md)                                                                                   |
