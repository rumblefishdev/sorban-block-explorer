---
id: '0006'
title: 'Research: AWS CDK with Nx monorepo organization'
type: RESEARCH
status: completed
related_adr: []
related_tasks:
  ['0024', '0025', '0026', '0027', '0029', '0028', '0030', '0056', '0031']
tags: [priority-medium, effort-medium, layer-research]
milestone: 1
links:
  - https://docs.aws.amazon.com/cdk/v2/guide/best-practices.html
  - https://docs.aws.amazon.com/cdk/v2/guide/stacks.html
  - https://docs.aws.amazon.com/lambda/latest/dg/rust-package.html
  - https://github.com/cargo-lambda/cargo-lambda-cdk
  - https://github.com/aripalo/aws-cdk-github-oidc
  - https://github.com/corymhall/cdk-diff-action
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created from architecture docs decomposition'
  - date: 2026-03-26
    status: active
    who: stkrolikiewicz
    note: 'Research started'
  - date: 2026-03-26
    status: completed
    who: stkrolikiewicz
    note: >
      Research complete. 4 notes (3 R-, 1 S-), 5 archived sources, 8/8 AC met.
      15 architecture decisions documented. CDK APIs verified against aws-cdk-lib@2.244.0.
      Key decisions: 6-stack decomposition, pre-built Rust binary, TypeScript config module,
      cr.Provider migration pattern, CloudFront Function Basic Auth, cargo-lambda via pip3.
---

# Research: AWS CDK with Nx monorepo organization

## Summary

Investigate how to organize AWS CDK infrastructure within the Nx monorepo, including stack decomposition, asset bundling from Nx app build outputs, environment configuration patterns, schema migration strategies, and GitHub Actions integration. This research must produce a CDK project structure that manages all infrastructure components while remaining open-source redeployable.

## Status: Completed

## Research Notes

| Note                                                                     | Topic                                                                 |
| ------------------------------------------------------------------------ | --------------------------------------------------------------------- |
| [R-stack-decomposition.md](notes/R-stack-decomposition.md)               | 6-stack layer-based decomposition with dependency ordering            |
| [R-nx-cdk-integration.md](notes/R-nx-cdk-integration.md)                 | Nx build targets, asset flow, Rust Lambda bundling options            |
| [R-env-config-and-migrations.md](notes/R-env-config-and-migrations.md)   | TypeScript config module, secret handling, Custom Resource migrations |
| [S-cdk-architecture-decisions.md](notes/S-cdk-architecture-decisions.md) | Synthesis of all CDK architecture decisions                           |

## Key Findings

- **6 layer-based stacks:** Network → Storage → Compute → Delivery → Monitoring → CI (OIDC). Stateful/stateless split, deployment ordering matches architecture docs.
- **Nx integration:** Custom `run-commands` targets (`synth`, `deploy`, `diff`) with `dependsOn: ["^build"]`. No plugin needed.
- **Rust Lambda:** Pre-built binary via `cargo lambda build --release --arm64`, referenced via `Code.fromAsset()`. `cargo-lambda-cdk` as fallback.
- **TypeScript config module:** Typed `EnvironmentConfig` interface at `infra/aws-cdk/lib/config/`. Account ID from `CDK_DEFAULT_ACCOUNT`.
- **Schema migration:** CDK Custom Resource Lambda running Drizzle Kit, ordered via `addDependency()`.
- **GitHub Actions:** OIDC roles (per ADR-0001), `cdk diff` on PR via `corymhall/cdk-diff-action`, auto-deploy staging on merge, production approval gate.
- **OIDC construct:** `aws-cdk-github-oidc` library in CiStack.
- **ECR + SQS DLQ:** ECR for Galexie Docker images and SQS DLQ for failed Ledger Processor invocations, both in ComputeStack.
- **Provisioned concurrency:** Only API Lambda in production (start with 5). Rust Processor doesn't need it (100ms cold start).
- **Staging password:** CloudFront Function with Basic Auth (not Lambda@Edge). Credentials stored in Secrets Manager and injected at deploy time (or as pre-hashed values), avoiding synth-time secret resolution in synthesized templates.
- **Cross-compilation:** `cargo-lambda` with Docker in CI, Zig locally; install `cargo-lambda` via `pip3` or use its Docker image directly in GitHub Actions (no dedicated `cargo-lambda-action`).
- **`cdk.context.json`:** Must be committed for deterministic synthesis.

## Context

The infrastructure is defined as code using AWS CDK written in TypeScript. Within the Nx workspace, the CDK project lives at `infra/aws-cdk`. The CDK must manage a large set of AWS resources across three environments while integrating with the Nx build system for application artifacts.

### Components to Manage

The CDK stacks must provision and configure the following AWS resources:

- **VPC** -- single-AZ at launch (us-east-1a), with public edge and private runtime subnets
- **ECS Fargate** -- two task definitions: Galexie (continuous live ingestion) and backfill (batch/one-time historical import)
- **S3** -- two buckets: `stellar-ledger-data` (transient XDR storage with lifecycle rules) and `api-docs` (OpenAPI spec and documentation portal)
- **Lambda** -- three functions: Ledger Processor (S3-triggered), Event Interpreter (EventBridge-triggered every 5 min), NestJS API handlers (API Gateway-triggered)
- **RDS PostgreSQL** -- Single-AZ at launch, with RDS Proxy for connection pooling
- **API Gateway** -- public REST API with throttling, request validation, and response caching
- **CloudFront** -- static frontend delivery and api-docs hosting
- **WAF** -- attached to API Gateway and CloudFront for abuse protection
- **Route 53** -- DNS management for public endpoints
- **EventBridge** -- scheduler for Event Interpreter Lambda
- **Secrets Manager** -- database credentials and integration secrets
- **CloudWatch** -- dashboards, metrics, and alarms
- **X-Ray** -- distributed tracing for Lambda functions

### Three Environments

1. **Development** -- local PostgreSQL for local and CI workflows; no AWS resources deployed
2. **Staging** -- testnet data, separate RDS instance, password-protected web frontend at edge, lower concurrency/throttling limits, shorter retention windows (7-day S3 artifacts), non-paging alerts
3. **Production** -- mainnet data, KMS-backed encryption for RDS and S3, TLS enforced, PITR enabled, deletion protection, 30-day S3 artifact retention, full paging alerts, WAF sized for public traffic

### Schema Migration in CDK

Database schema migrations must run before Lambda code deployment. The architecture specifies this as either a CDK custom resource or a CodeBuild step in the deployment pipeline. The migration mechanism must be part of the CDK deployment flow, not a manual operational step.

### Nx Integration

Nx app build outputs (compiled Lambda handlers, bundled frontend assets) must flow into CDK asset bundling. The CDK deployment must reference Nx-built artifacts rather than rebuilding application code during `cdk deploy`. This means the CDK project must understand Nx output paths and the Nx build graph.

### GitHub Actions Integration

The deployment pipeline uses GitHub Actions with `cdk deploy`. The CI/CD model must support environment-specific deployments (staging, production) with appropriate approval gates for production.

### Open-Source Redeployability

The main design explicitly assumes the full stack can be redeployed by third parties in a fresh AWS account. This means no hard-coded AWS account IDs, no internal-only service dependencies, and all configuration must be parameterizable.

## Research Questions

- How should CDK stacks be decomposed? One stack per resource group (networking, compute, storage, delivery) or one stack per environment? What are the deployment ordering dependencies?
- How should Nx build outputs be referenced from CDK? Should CDK use `aws_lambda.Code.fromAsset()` pointing to Nx `dist/` output, or is there a better pattern?
- How should environment-specific configuration be managed in CDK? Context values, environment variables, or a separate config file?
- What is the best approach for schema migration in CDK: custom resource Lambda that runs Drizzle Kit migrations, or a CodeBuild step? What are the rollback implications?
- How should the CDK project be structured as an Nx project with its own build/deploy targets?
- How should GitHub Actions workflows be structured for `cdk diff` on PRs and `cdk deploy` on merge?
- How can the CDK project avoid hard-coded account IDs for open-source redeployability? CDK environment synthesis patterns?
- What is the recommended pattern for managing Lambda provisioned concurrency configuration per environment?
- How should S3 lifecycle rules differ between staging (7-day) and production (30-day)?
- How should the staging web frontend password protection be implemented at the CloudFront/edge level?

## Research Questions → Answer Location

| #   | Question                        | Answered In                                                                                                                |
| --- | ------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| 1   | CDK stack decomposition         | [R-stack-decomposition](notes/R-stack-decomposition.md)                                                                    |
| 2   | Nx build outputs → CDK          | [R-nx-cdk-integration § Nx → CDK Asset Flow](notes/R-nx-cdk-integration.md#nx--cdk-asset-flow)                             |
| 3   | Environment configuration       | [R-env-config § Environment Configuration](notes/R-env-config-and-migrations.md#environment-configuration-approach)        |
| 4   | Schema migration approach       | [R-env-config § Schema Migration](notes/R-env-config-and-migrations.md#schema-migration-strategy)                          |
| 5   | CDK project structure in Nx     | [R-nx-cdk-integration § CDK as Nx Project](notes/R-nx-cdk-integration.md#cdk-as-an-nx-project)                             |
| 6   | GitHub Actions + CDK            | [R-env-config § GitHub Actions](notes/R-env-config-and-migrations.md#github-actions-integration)                           |
| 7   | Open-source redeployability     | [R-env-config § Account ID Handling](notes/R-env-config-and-migrations.md#account-id-handling-open-source-redeployability) |
| 8   | Provisioned concurrency per env | [R-env-config § Provisioned Concurrency](notes/R-env-config-and-migrations.md#provisioned-concurrency-per-environment)     |
| 9   | S3 lifecycle per env            | [R-env-config § S3 Lifecycle Rules](notes/R-env-config-and-migrations.md#s3-lifecycle-rules-per-environment)               |
| 10  | Staging password protection     | [R-env-config § Staging Password](notes/R-env-config-and-migrations.md#staging-frontend-password-protection)               |

## Acceptance Criteria

- [x] Recommended CDK stack decomposition with dependency ordering
- [x] Nx-to-CDK asset bundling pattern documented
- [x] Environment configuration approach documented (dev/staging/production)
- [x] Schema migration strategy selected with CDK integration pattern
- [x] CDK project structure within Nx workspace documented
- [x] GitHub Actions workflow structure for CDK deployment documented
- [x] Open-source redeployability approach confirmed (no hard-coded account IDs)
- [x] Environment-specific resource configuration patterns documented (sizing, encryption, retention)

## Design Decisions

### From Plan

1. **6-stack layer-based decomposition:** Network → Storage → Compute → Delivery → Monitoring → CI. Matches architecture docs deployment ordering.
2. **TypeScript config module:** Typed `EnvironmentConfig` at `infra/aws-cdk/lib/config/`. Per architecture docs requirement for `infra/aws-cdk/config/*`.
3. **CDK Custom Resource for migrations:** Drizzle Kit Lambda with `cr.Provider`, ordered via `addDependency()`. Per architecture docs "migrations before Lambda code".
4. **OIDC deploy roles:** `aws-cdk-github-oidc` construct in CiStack. Per ADR-0001.

### Emerged

5. **Pre-built Rust binary over `cargo-lambda-cdk`:** Task didn't specify bundling approach. Chose pre-built binary with `Code.fromAsset()` because Nx should manage ALL builds including Rust.
6. **CloudFront Function over Lambda@Edge for staging password:** Task asked "how should staging password protection be implemented". Chose CF Function (6x cheaper, sub-ms) over Lambda@Edge.
7. **`cargo-lambda/cargo-lambda-action` does not exist (404):** Discovered during verification. Replaced with `pip3 install cargo-lambda` + `dtolnay/rust-toolchain`.
8. **ECR repository in ComputeStack:** Task didn't mention ECR. Added because Galexie needs Docker image registry.
9. **SQS DLQ in ComputeStack:** Task didn't mention DLQ. Added per backlog task 0033 requirement.
10. **ACM certificates in DeliveryStack:** Not in task scope but required for CloudFront/API Gateway HTTPS.
11. **`cdk.context.json` must be committed:** Not in task requirements. Added because non-deterministic synthesis is a real risk.

## Issues Encountered

- **`cargo-lambda-cdk` version mismatch:** Initially wrote `^0.1.0` — doesn't exist. Actual latest is `0.0.36`. Fixed after npm verification.
- **`AwsCustomResource` vs `Provider` pattern:** Initially used `AwsCustomResource` for migration Lambda — wrong. `AwsCustomResource` is for AWS SDK calls. `Provider` + `CustomResource` is correct for invoking Lambda. Fixed after CDK API review.
- **Nx project graph failure:** `npx nx format` failed with "Failed to process project graph". Required `npx nx reset` to fix corrupted daemon state.

## Notes

- The infrastructure starts with Single-AZ in us-east-1a. The CDK structure should make it straightforward to expand to Multi-AZ when SLA requirements justify it.
- The development environment uses local PostgreSQL and does not deploy AWS resources -- CDK only manages staging and production.
- CloudWatch alarms have specific thresholds documented in the infrastructure overview (e.g., Galexie lag >60s, RDS CPU >70% for 5min, API 5xx >0.5%). These should be parameterizable per environment.
