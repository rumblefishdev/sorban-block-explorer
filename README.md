# Soroban Block Explorer

[**Backlog Board**](https://rumblefishdev.github.io/soroban-block-explorer/)

Nx + TypeScript monorepo bootstrap for a Soroban-first Stellar block explorer.

This repository starts from the official `nrwl/typescript-template` foundation and adapts
it to the planned product architecture:

- `web` for the frontend explorer UI
- `crates/api` for the public REST API (Rust/axum)
- `crates/indexer` for ledger ingestion entrypoints (Rust)
- `infra` for infrastructure as code (AWS CDK)
- `libs/ui` for shared frontend components

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
web/
crates/
  api/
  indexer/
  xdr-parser/
infra/
libs/
  ui/
docs/
  architecture/
```

## Current Status

The workspace contains:

- root Nx / TypeScript / ESLint / Prettier bootstrap
- `web` — React 19 + Vite SPA with MUI, React Router, and TanStack Query
- `libs/ui` — shared React component library (Vite lib mode)
- `infra` — AWS CDK infrastructure stacks
- `crates/` — Rust backend (api, indexer, xdr-parser)
- architecture docs aligned with the reviewed technical design

Backend: Rust (axum + utoipa + sqlx), deployed as Lambda via cargo-lambda (per ADR 0005).
They will be introduced as dedicated follow-up steps.

## Infrastructure

AWS infrastructure is managed with CDK (TypeScript) in `infra/`.

### Prerequisites

- AWS CLI configured with a named profile:
  ```bash
  aws configure --profile soroban-explorer
  ```
- Set the profile in your shell:
  ```bash
  export AWS_PROFILE=soroban-explorer
  ```

### First-time setup

Bootstrap CDK on the AWS account (once per account + region):

```bash
npm run infra:bootstrap
```

### Commands

```bash
npm run infra:diff:staging        # Preview changes
npm run infra:deploy:staging      # Deploy to AWS
npm run infra:synth:staging       # Generate CloudFormation template
```

Replace `staging` with `production` for production deployments.

### CI deploy (staging)

Staging deploys via GitHub Actions, triggered by git tags (see [ADR 0009](lore/2-adrs/0009_staging-deploy-trigger-strategy.md)):

```bash
./scripts/staging-deploy.sh              # tag and deploy current HEAD on develop
./scripts/staging-deploy.sh <commit-sha> # rollback: deploy a specific commit
```

The script creates a `staging-YYYY.MM.DD-N` tag and pushes it. The deploy workflow runs automatically. Manual redeploy without a tag: `gh workflow run deploy-staging.yml --ref develop`.

Or use the Makefile directly from `infra/`:

```bash
make diff-staging
make deploy-staging
make deploy-staging-network    # Deploy single stack
```
