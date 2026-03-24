# Soroban Block Explorer

Nx + TypeScript monorepo bootstrap for a Soroban-first Stellar block explorer.

This repository starts from the official `nrwl/typescript-template` foundation and adapts
it to the planned product architecture:

- `apps/web` for the frontend explorer UI
- `apps/api` for the public REST API
- `apps/indexer` for ledger ingestion entrypoints
- `apps/workers` for background processing and interpretation jobs
- `infra/aws-cdk` for infrastructure as code
- `libs/domain`, `libs/shared`, `libs/ui` for reusable internal code

## Quick Start

```bash
nvm use
npm install
npm run lint
npm run build
npm run typecheck
```

## Workspace Layout

```text
apps/
  api/
  indexer/
  web/
  workers/
infra/
  aws-cdk/
libs/
  domain/
  shared/
  ui/
docs/
  architecture/
```

## Current Status

This is an initialization commit. The workspace contains:

- root Nx / TypeScript / ESLint / Prettier bootstrap
- minimal package-based project skeletons for the core bounded contexts
- starter architecture docs aligned with the reviewed technical design

Application framework plugins such as React, NestJS, and AWS-specific runtime code are not
added yet. They should be introduced as dedicated follow-up steps so the workspace history
stays clean and each architectural decision is explicit.
