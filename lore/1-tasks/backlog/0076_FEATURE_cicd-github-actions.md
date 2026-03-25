---
id: '0076'
title: 'CI/CD pipeline: GitHub Actions workflows'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0021']
tags: [priority-high, effort-medium, layer-infra]
links:
  - docs/architecture/infrastructure/infrastructure-overview.md
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# CI/CD pipeline: GitHub Actions workflows

## Summary

Define GitHub Actions workflows for the full CI/CD pipeline: Nx-based build/lint/test using affected commands, environment-specific deployment with CDK, enforced deployment ordering (DB migrations before Lambda code), ECR image builds for Galexie, and manual production approval via GitHub Environments. Staging auto-deploys on merge to main; production requires explicit approval.

## Status: Backlog

**Current state:** Not started. Database migration framework (task 0021) must be in place for the migration-first deployment ordering.

## Context

The CI/CD pipeline is the delivery mechanism for the entire block explorer. It handles both application code and infrastructure updates through a single GitHub Actions workflow. The pipeline enforces critical deployment ordering: database schema migrations must succeed before new application code is deployed.

The monorepo uses Nx for task orchestration, so CI builds leverage Nx affected commands to only build, lint, and test what changed.

### Source Code Location

- `.github/workflows/`

## Implementation Plan

### Step 1: CI Build Workflow

Define the continuous integration workflow triggered on pull requests and pushes to main:

**Build steps:**

1. Checkout code
2. Setup Node.js
3. Install dependencies (pnpm)
4. Run `pnpm nx affected --target=lint` -- lint only affected projects
5. Run `pnpm nx affected --target=test` -- test only affected projects
6. Run `pnpm nx affected --target=build` -- build only affected projects

**Nx Cloud cache:** If Nx Cloud is configured, CI builds benefit from remote caching. This is optional but recommended for large teams.

**Status checks:** All three affected commands must pass before merge is allowed.

### Step 2: Deployment Ordering Enforcement

Define strict deployment ordering:

1. **Run DB migrations** against the target RDS instance (staging or production)
   - Use the migration framework from task 0021
   - Connect via RDS Proxy
   - If migration fails: ABORT the entire deployment. Do not deploy new code.
2. **Only after migration success:** deploy new Lambda code + update API Gateway + update other CDK resources

This ordering is enforced in the workflow job dependencies (migration job must succeed before deploy job runs).

### Step 3: Staging Deployment

Define auto-deployment to staging:

- Trigger: successful merge to `main` branch after CI passes
- Environment: GitHub Environment `staging` with staging-specific secrets
- Steps:
  1. Build all applications
  2. Run DB migrations against staging RDS
  3. `cdk deploy` with staging configuration
  4. Build and push Galexie container image to ECR (if changed)
  5. Update ECS Fargate service (if container image changed)

### Step 4: Production Deployment

Define manual deployment to production:

- Trigger: manual workflow dispatch or GitHub Environment approval gate
- Environment: GitHub Environment `production` with production-specific secrets
- Steps:
  1. `cdk diff` step before approval -- outputs changeset for review
  2. **Manual approval** via GitHub Environment protection rules
  3. After approval: build all applications
  4. Run DB migrations against production RDS
  5. `cdk deploy` with production configuration
  6. Build and push Galexie container image to ECR (if changed)
  7. Update ECS Fargate service (if container image changed)

### Step 5: GitHub Environments Configuration

Define GitHub Environments:

**staging:**

- Secrets: AWS credentials (access key or OIDC), CDK context values, RDS connection info
- No approval gate (auto-deploy)
- Deployment branch: `main`

**production:**

- Secrets: AWS credentials, CDK context values, RDS connection info
- Required reviewers for approval gate
- Deployment branch: `main`

Each environment has its own set of secrets so staging and production AWS accounts are fully isolated.

### Step 6: ECR Image Build

Define Galexie container image build and push:

- Build the Docker image for the Galexie/backfill container
- Tag with git SHA for traceability
- Push to ECR repository (defined in task 0078)
- Only build when relevant files change (Dockerfile, Galexie-related source)

### Step 7: Rollback Strategy

Define rollback procedure:

- Re-run the previous successful workflow to redeploy the last known good version
- CDK deploy with previous artifact versions
- Database rollback migrations if available (down migrations from task 0021)
- Document the rollback procedure in workflow comments

### Step 8: Protocol Upgrade Workflow

Document the process for Stellar protocol upgrades:

- Update `@stellar/stellar-sdk` dependency
- Run integration tests with known ledger fixtures (sample XDR files from before and after the protocol change)
- Deploy through normal staging -> production pipeline
- No special CI/CD changes needed; the normal pipeline handles it

## Acceptance Criteria

- [ ] CI workflow runs lint, test, and build using Nx affected commands
- [ ] CI must pass before merge to main is allowed
- [ ] Staging auto-deploys on merge to main after CI passes
- [ ] Production requires manual approval via GitHub Environments
- [ ] `cdk diff` is output before production approval for changeset review
- [ ] DB migrations run BEFORE Lambda code deployment in both environments
- [ ] Migration failure aborts the entire deployment
- [ ] Each environment (staging, production) has its own GitHub Environment with separate secrets
- [ ] Galexie container image is built and pushed to ECR with git SHA tag
- [ ] Rollback is possible by re-running previous successful workflow
- [ ] Nx Cloud cache integration is documented (optional)
- [ ] GitHub Actions authenticates to AWS via OIDC role assumption (no long-lived AWS access keys in GitHub secrets)
- [ ] Staging and production deployments assume separate AWS IAM roles with environment-scoped permissions
- [ ] CI/CD AWS IAM roles follow least-privilege principle; each role can only deploy to its target environment
- [ ] GitHub Actions workflows reference secret names only; no secret values embedded in workflow YAML files
- [ ] Protocol upgrade path documented: update @stellar/stellar-sdk, run integration tests with pre/post-upgrade XDR fixtures, deploy via normal pipeline

## Notes

- AWS credential management in GitHub Actions can use OIDC (preferred for security) or access keys stored as GitHub secrets. OIDC avoids long-lived credentials.
- The `cdk diff` step for production is critical for change visibility. Reviewers should check the diff before approving.
- ECR image builds should be cached (Docker layer caching) to reduce build times.
- The pipeline should be fast enough that staging deployments complete within a few minutes of merge.
- Protocol upgrades are infrequent. The integration test step with known ledger fixtures ensures the new SDK version correctly parses both old and new format ledgers.
