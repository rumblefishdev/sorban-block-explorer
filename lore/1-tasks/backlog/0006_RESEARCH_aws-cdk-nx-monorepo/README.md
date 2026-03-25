---
id: '0006'
title: 'Research: AWS CDK with Nx monorepo organization'
type: RESEARCH
status: backlog
related_adr: []
related_tasks:
  ['0060', '0061', '0062', '0063', '0064', '0065', '0066', '0067', '0068']
tags: [priority-medium, effort-medium, layer-research]
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created from architecture docs decomposition'
---

# Research: AWS CDK with Nx monorepo organization

## Summary

Investigate how to organize AWS CDK infrastructure within the Nx monorepo, including stack decomposition, asset bundling from Nx app build outputs, environment configuration patterns, schema migration strategies, and GitHub Actions integration. This research must produce a CDK project structure that manages all infrastructure components while remaining open-source redeployable.

## Status: Backlog

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

## Acceptance Criteria

- [ ] Recommended CDK stack decomposition with dependency ordering
- [ ] Nx-to-CDK asset bundling pattern documented
- [ ] Environment configuration approach documented (dev/staging/production)
- [ ] Schema migration strategy selected with CDK integration pattern
- [ ] CDK project structure within Nx workspace documented
- [ ] GitHub Actions workflow structure for CDK deployment documented
- [ ] Open-source redeployability approach confirmed (no hard-coded account IDs)
- [ ] Environment-specific resource configuration patterns documented (sizing, encryption, retention)

## Notes

- The infrastructure starts with Single-AZ in us-east-1a. The CDK structure should make it straightforward to expand to Multi-AZ when SLA requirements justify it.
- The development environment uses local PostgreSQL and does not deploy AWS resources -- CDK only manages staging and production.
- CloudWatch alarms have specific thresholds documented in the infrastructure overview (e.g., Galexie lag >60s, RDS CPU >70% for 5min, API 5xx >0.5%). These should be parameterizable per environment.
