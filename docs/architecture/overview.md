# Architecture Overview

This repository follows the reviewed design for a Soroban-first Stellar block explorer.

## Planned Runtime Components

- `apps/web`: public explorer frontend
- `apps/api`: NestJS REST API reading from PostgreSQL
- `apps/indexer`: ingestion entrypoints for ledger processing
- `apps/workers`: scheduled/background jobs such as event interpretation
- `infra/aws-cdk`: AWS infrastructure definitions

## Shared Code Boundaries

- `libs/shared`: generic cross-cutting code with no business dependencies
- `libs/domain`: block explorer domain models and shared business logic
- `libs/ui`: frontend-facing presentation primitives and UI composition helpers

## Data Flow

1. Galexie writes `LedgerCloseMeta` payloads to S3.
2. Ledger processing code ingests and normalizes chain data into PostgreSQL.
3. Background workers enrich recent records with human-readable interpretations.
4. The public API serves normalized data to the frontend.
5. The frontend polls for fresh network state and renders explorer views.

## Bootstrap Scope

This initial workspace does not implement the runtime stack yet. It only establishes:

- project boundaries
- TypeScript workspace wiring
- Nx task discovery
- lint/typecheck/build entrypoints
- repo-level docs and CI baseline
