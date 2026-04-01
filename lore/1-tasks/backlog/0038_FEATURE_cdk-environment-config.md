---
id: '0038'
title: 'CDK: environment-specific configuration (dev/staging/prod)'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0006']
tags: [priority-high, effort-medium, layer-infra]
milestone: 1
links:
  - docs/architecture/infrastructure/infrastructure-overview.md
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# CDK: environment-specific configuration (dev/staging/prod)

## Summary

Define a centralized configuration module in CDK that provides environment-specific values for all infrastructure resources. Three profiles (development, staging, production) control database sizing, Lambda concurrency, caching behavior, encryption, lifecycle rules, alarm thresholds, and access controls. All AWS account-specific values are parameterized for redeployability.

## Status: Backlog

**Current state:** Not started. This task is consumed by all other CDK tasks (0031-0037, 0039, 0040) for environment-specific values.

## Context

The block explorer deploys to three environments with different operational profiles. Rather than scattering environment-specific values across individual CDK constructs, a single configuration module provides all environment-dependent parameters. This ensures consistency and makes the full stack redeployable to any AWS account.

### Source Code Location

- `infra/aws-cdk/lib/config/`

## Implementation Plan

### Step 1: Configuration Module Structure

Create a configuration module that exports environment-specific values based on a profile selector (CDK context variable or environment variable):

```
getConfig(environment: 'development' | 'staging' | 'production'): EnvironmentConfig
```

All values are in this single module. Individual CDK stacks import from here.

### Step 2: Development Profile

Development uses local resources only, no AWS infrastructure:

- Local PostgreSQL database (Docker Compose or native)
- No AWS Lambda, ECS, S3, RDS, or CloudFront
- No CDK deployment needed
- Used for local development and CI unit tests

### Step 3: Staging Profile

Staging mirrors production topology with reduced resources:

**Compute:**

- Lower Lambda concurrency limits
- Lower API Gateway throttling rates
- Smaller Lambda memory allocations where applicable

**Storage:**

- Smaller RDS instance class (e.g., db.t3.medium)
- S3 lifecycle: 7-day retention on stellar-ledger-data
- Shorter CloudWatch log retention

**Network:**

- Testnet Captive Core passphrase for Galexie
- Testnet data only

**Caching:**

- Smaller API Gateway cache allocation
- Shorter cache TTLs

**Security:**

- SSE-S3 encryption acceptable (not KMS)
- TLS optional on database connections
- Optional IP allowlists for restricted access
- Password protection on CloudFront (via CloudFront Functions basic auth)

**Observability:**

- Non-paging alarms (email/Slack only)
- Potentially relaxed alarm thresholds (higher error rates tolerated)
- Shorter log and trace retention

### Step 4: Production Profile

Production is the public-facing baseline:

**Compute:**

- Full provisioned concurrency on API Lambda
- Production-level API Gateway throttling
- Production memory allocations

**Storage:**

- Production-sized RDS instance
- S3 lifecycle: 30-day retention on stellar-ledger-data
- Longer CloudWatch log retention
- Automated RDS backups + PITR + deletion protection

**Network:**

- Mainnet Captive Core passphrase for Galexie
- Mainnet data

**Caching:**

- Full API Gateway cache allocation
- Production TTLs (long for immutable, 5-15s for mutable)

**Security:**

- KMS-backed encryption on RDS and S3 ledger-data bucket
- TLS enforced on database connections (`rds.force_ssl = 1`)
- No IP allowlists (public access via WAF)
- No password protection on CloudFront

**Observability:**

- Full paging alarms via SNS/PagerDuty
- Strict alarm thresholds as documented in task 0036
- Longer log and trace retention

### Step 5: Parameterized AWS Values

All AWS account-specific values must be parameterized, not hard-coded:

- AWS account ID
- AWS region
- VPC CIDR ranges
- Domain names and hosted zone IDs
- ECR repository URIs
- KMS key ARNs (if using existing keys)

These are provided via CDK context, environment variables, or CDK context JSON files.

### Step 6: Profile Selection Mechanism

Support profile selection via:

- CDK context variable: `--context environment=staging`
- Environment variable: `CDK_ENVIRONMENT=production`
- Default: development (safest default)

The CI/CD pipeline (task 0039) passes the appropriate profile for each deployment target.

## Acceptance Criteria

- [ ] Single configuration module exports all environment-specific values
- [ ] Development profile: local PostgreSQL, no AWS resources
- [ ] Staging profile: testnet, smaller DB, 7-day S3 lifecycle, lower concurrency, non-paging alarms, SSE-S3, optional TLS, password-protected CloudFront
- [ ] Production profile: mainnet, production DB, 30-day S3 lifecycle, full concurrency, paging alarms, KMS encryption, enforced TLS, public CloudFront
- [ ] All alarm thresholds are environment-configurable
- [ ] All AWS account-specific values are parameterized (not hard-coded)
- [ ] Profile selection works via CDK context or environment variable
- [ ] Configuration module is consumed by all CDK stacks
- [ ] Stack is redeployable to any AWS account by changing parameters only
- [ ] Non-secret environment configuration files stored in infra/aws-cdk/config/ (committed to git)
- [ ] No .env.prod, .env.staging, or similar secret-containing files in the repository; no secret values in CDK context files, TypeScript constants, or workflow YAML
- [ ] CDK stacks consume only secret references (ARNs, parameter names); no hard-coded secret values in any CDK code
- [ ] Staging stellar-ledger-data S3 lifecycle: minimum 7-day retention
- [ ] Production stellar-ledger-data S3 lifecycle: minimum 30-day retention
- [ ] Staging profile specifies shorter CloudWatch log retention and X-Ray trace retention than production
- [ ] Staging profile specifies smaller API Gateway cache size and shorter cache TTLs compared to production
- [ ] Staging profile supports optional IP allowlists and reduced DNS discoverability as additional access controls
- [ ] Infrastructure self-contained within a single AWS sub-account with no cross-account resource dependencies for core runtime
- [ ] Infrastructure redeployable to any AWS account by changing parameters only; no hidden dependency on internal-only services
- [ ] Configuration enforces deployment into a dedicated AWS sub-account

## Notes

- The configuration module should use TypeScript interfaces to ensure type safety across all CDK stacks that consume it.
- Default values should be safe (development profile). Production values should require explicit opt-in.
- Staging and production share the same CDK stacks; only configuration values differ. No conditional CDK construct inclusion based on environment.
- If a value is the same across all environments, it belongs in the CDK stack definition, not in the config module.
- The development profile existence is important for the README and onboarding -- it documents that local development does not require AWS access.
