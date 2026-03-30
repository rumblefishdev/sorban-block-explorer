---
id: '0032'
title: 'CDK: RDS PostgreSQL, RDS Proxy, S3 buckets, Secrets Manager'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0006', '0031']
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

# CDK: RDS PostgreSQL, RDS Proxy, S3 buckets, Secrets Manager

## Summary

Define the storage infrastructure using CDK: RDS PostgreSQL (Single-AZ at launch) with RDS Proxy for connection pooling, two S3 buckets (stellar-ledger-data for XDR files, api-docs for documentation portal), and Secrets Manager for database credentials with automatic rotation. All Lambda connections to the database go through RDS Proxy exclusively.

## Status: Backlog

**Current state:** Not started. Depends on VPC/networking (task 0031) for subnet and security group placement.

## Context

The block explorer owns its full PostgreSQL database. RDS PostgreSQL is the single storage backend for all explorer data. RDS Proxy is mandatory for Lambda connections because Lambda burst execution can exhaust database connection limits without pooling.

Two S3 buckets serve distinct purposes:

- `stellar-ledger-data`: receives LedgerCloseMeta XDR files from Galexie and backfill, triggers the Ledger Processor Lambda
- `api-docs`: hosts the OpenAPI documentation portal, served through CloudFront

### Source Code Location

- `infra/aws-cdk/lib/storage/`

## Implementation Plan

### Step 1: RDS PostgreSQL Instance

Define the RDS PostgreSQL instance:

- Engine: PostgreSQL (latest stable version compatible with partitioning and JSONB features)
- Deployment: Single-AZ at launch (us-east-1a)
- Instance class: environment-specific (smaller for staging, production-sized for prod, defined in task 0038)
- Storage: GP3, auto-scaling enabled
- VPC placement: private subnet (from task 0031)
- Security group: RDS SG (from task 0031), allowing inbound from Lambda SG

**Production hardening:**

- Encryption at rest: KMS-backed (customer-managed key)
- TLS enforced: `rds.force_ssl = 1` parameter
- Automated backups enabled
- Point-in-time recovery (PITR) enabled
- Deletion protection enabled
- Backup retention: 7+ days

**Design note:** A read replica is NOT provisioned at launch. Add one when CPU exceeds the monitoring threshold (defined in task 0036).

### Step 2: RDS Proxy

Define RDS Proxy for connection pooling:

- ALL Lambda connections go through RDS Proxy, never direct to RDS
- Proxy uses Secrets Manager for database credentials
- Proxy handles credential rotation transparently (Lambdas do not need redeployment on rotation)
- Target: the RDS PostgreSQL instance
- VPC placement: same private subnet
- Security group: allows inbound from Lambda SG

### Step 3: S3 Bucket - stellar-ledger-data

Define the XDR storage bucket:

- Bucket name: environment-prefixed (e.g., `prod-stellar-ledger-data`)
- Key prefix: `ledgers/` with pattern `{seq_start}-{seq_end}.xdr.zstd`
- S3 event notification: PutObject with prefix filter (`ledgers/`) and suffix filter (`.xdr.zstd`) triggers the Ledger Processor Lambda (defined in task 0033)
- Lifecycle rules:
  - Production: 30-day retention, then transition to cheaper storage or delete
  - Staging: 7-day retention
- Encryption:
  - Production: KMS-backed SSE (SSE-KMS)
  - Staging: SSE-S3 acceptable
- Versioning: disabled (objects are immutable, identified by ledger sequence)
- Block public access: enabled

### Step 4: S3 Bucket - api-docs

Define the documentation portal bucket:

- Hosts OpenAPI specification and documentation static files
- Fronted by CloudFront (configured in task 0035)
- Encryption:
  - Production: KMS-backed SSE
  - Staging: SSE-S3 acceptable
- Block public access: enabled (access via CloudFront OAI/OAC only)

### Step 5: Secrets Manager

Define database credential management:

- Store RDS master credentials in Secrets Manager
- Automatic rotation: 30-day cycle for production
- RDS Proxy references the secret for transparent credential access
- Lambda functions reference the secret ARN (via environment variable) for RDS Proxy connection string construction
- No credentials in source code, environment variables contain only the secret ARN

### Step 6: Encryption Key Management

For production:

- Create or reference a KMS customer-managed key
- Used for: RDS encryption at rest, S3 SSE-KMS on both buckets
- Key policy grants access to RDS service, S3 service, and relevant IAM roles

## Acceptance Criteria

- [ ] RDS PostgreSQL instance is defined in the private subnet with appropriate engine and storage
- [ ] RDS Proxy is defined and configured for connection pooling with Secrets Manager integration
- [ ] All Lambda database access is routed through RDS Proxy (no direct RDS connections)
- [ ] stellar-ledger-data S3 bucket is defined with correct prefix/suffix event notification filter
- [ ] S3 event notification triggers the Ledger Processor Lambda on PutObject
- [ ] Lifecycle rules: 30-day production, 7-day staging
- [ ] api-docs S3 bucket is defined with public access blocked
- [ ] Secrets Manager stores RDS credentials with automatic 30-day rotation for production
- [ ] Production: KMS encryption on RDS and both S3 buckets, TLS enforced on RDS
- [ ] Staging: SSE-S3 acceptable, TLS optional
- [ ] Production: automated backups, PITR, deletion protection enabled
- [ ] No read replica at launch (documented as future addition based on CPU threshold)
- [ ] RDS definition structured so Single-AZ to Multi-AZ promotion requires only a configuration change
- [ ] Staging and production use separate RDS instances (no shared database)
- [ ] S3 lifecycle retention rationale documented in CDK code comments: supports XDR replay and incident validation

## Notes

- RDS Proxy adds ~1ms latency per connection but prevents connection exhaustion under Lambda burst. This is a required tradeoff.
- The S3 event notification filter (prefix + suffix) prevents the Lambda from being triggered by non-XDR files in the bucket.
- Secrets Manager automatic rotation with RDS Proxy means Lambda functions do not need redeployment when credentials rotate. The proxy handles the rotation transparently.
- All AWS account-specific values (account ID, region) must be parameterized, not hard-coded. The infrastructure must be redeployable to any AWS account.
