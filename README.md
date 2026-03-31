# Soroban Block Explorer

[**Backlog Board**](https://rumblefishdev.github.io/soroban-block-explorer/)

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

The workspace contains:

- root Nx / TypeScript / ESLint / Prettier bootstrap
- `apps/web` — React 19 + Vite SPA with MUI, React Router, and TanStack Query
- `libs/ui` — shared React component library (Vite lib mode)
- `libs/domain` — domain types for all explorer entities
- `libs/shared` — cross-cutting error types and handlers
- `apps/api`, `apps/indexer`, `apps/workers`, `infra/aws-cdk` — project skeletons
- architecture docs aligned with the reviewed technical design

Backend framework plugins (NestJS) and AWS-specific runtime code are not added yet.
They will be introduced as dedicated follow-up steps.

## Infrastructure

AWS infrastructure is managed with CDK (TypeScript) in `infra/aws-cdk/`.

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

Or use the Makefile directly from `infra/aws-cdk/`:

```bash
make diff-staging
make deploy-staging
make deploy-staging-network    # Deploy single stack
```
