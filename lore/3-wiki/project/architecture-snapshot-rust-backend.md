# Architecture Snapshot: Rust Backend (2026-04-01)

> **⚠ Pre-ADR 0012 snapshot.** This document describes the backend stack and data access
> patterns as of 2026-04-01, BEFORE [ADR 0012](../../2-adrs/0012_zero-upsert-schema-full-fk-graph.md)
> (zero-upsert schema, S3 offload, activity projections, `created_at` partitioning on
> `operations`). Backend stack (axum, sqlx, utoipa) is still valid post-ADR 0012; DB access
> patterns and response-source mapping described below are NOT. Refresh after ADR 0012
> lands and schema migration (task 0142) completes. Audited 2026-04-17 per task 0140.

> Created as part of task 0093 (backlog cleanup after ADR 0005).

## Backend Stack (per ADR 0005, research 0092)

| Layer          | Technology            | Version   |
| -------------- | --------------------- | --------- |
| Web framework  | axum                  | 0.8       |
| Lambda runtime | lambda_http           | 1.1       |
| OpenAPI        | utoipa + utoipa-axum  | 5.4 / 0.2 |
| Database       | sqlx (direct, no ORM) | 0.8       |
| Middleware     | tower-http            | 0.6       |
| Migrations     | sqlx-cli (plain SQL)  | —         |
| Build tool     | cargo-lambda          | 1.9       |
| CDK construct  | cargo-lambda-cdk      | —         |

## Monorepo Structure (target, after tasks 0094 + 0095)

```
soroban-block-explorer/
├── Cargo.toml              # Rust workspace root
├── crates/
│   ├── api/                # axum REST API Lambda
│   ├── indexer/            # Ledger Processor Lambda
│   ├── xdr-parser/         # XDR deserialization library
│   ├── db/                 # sqlx pool, queries, migrations
│   └── domain/             # shared types, errors, config
├── web/                    # React frontend
├── infra/                  # AWS CDK
├── libs/
│   ├── api-types/          # Generated TS types from OpenAPI
│   ├── domain/             # TS domain types (frontend)
│   ├── shared/             # TS utils (frontend)
│   └── ui/                 # React components
├── nx.json
└── package.json
```

## Key Decisions

- **No ORM** — API is read-only, sqlx `query_as!` with compile-time SQL validation
- **utoipa over aide** — better OpenAPI docs (examples, deprecation, tags)
- **sqlx migrations** — plain SQL files, Drizzle Kit dropped
- **5 Rust crates** — api, indexer, xdr-parser, db, domain
- **OpenAPI → TypeScript codegen** — @hey-api/openapi-ts for shared types

## ADR Status

| ADR  | Title                               | Status   |
| ---- | ----------------------------------- | -------- |
| 0002 | Rust Ledger Processor               | accepted |
| 0004 | Rust-only XDR parsing               | accepted |
| 0005 | Rust-only backend (API + Processor) | accepted |

## Completed Tasks

- 0092: Research Rust API stack (framework, ORM, deployment)
- 0093: Backlog cleanup (NestJS → Rust transition)

## Pending Tasks (blockers)

- 0094: Scaffold Cargo workspace (blocks all Rust implementation)
- 0095: Monorepo restructure (web/ top-level, flatten infra/)
- 0096: OpenAPI → TypeScript codegen
