---
id: '0033'
title: 'CDK: Lambda functions + SQS DLQ'
type: FEATURE
status: active
related_adr: ['0005']
related_tasks: ['0006', '0031', '0032', '0092', '0097']
tags: [priority-high, effort-medium, layer-infra]
milestone: 1
links:
  - docs/architecture/infrastructure/infrastructure-overview.md
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
  - date: 2026-03-31
    status: backlog
    who: stkrolikiewicz
    note: 'Updated per ADR 0005: Node.js Lambda → Rust Lambda (cargo-lambda-cdk RustFunction)'
  - date: 2026-04-01
    status: active
    who: fmazur
    note: 'Task activated'
  - date: 2026-04-01
    status: active
    who: fmazur
    note: 'Scope narrowed: API Gateway, WAF, usage plans split to task 0097'
---

# CDK: Lambda functions + SQS DLQ

## Summary

Define the three Lambda functions (API, Ledger Processor, Event Interpreter) and the SQS Dead Letter Queue using CDK. All Lambdas run as Rust binaries on ARM64/Graviton2 via cargo-lambda-cdk RustFunction. The API Lambda has provisioned concurrency. The Ledger Processor is triggered by S3 PutObject with a DLQ. The Event Interpreter is triggered by EventBridge (configured in task 0037).

API Gateway, WAF attachment, and usage plans are handled separately in task 0097.

## Status: Active

**Current state:** Not started. Dependencies on VPC/networking (task 0031) and storage (task 0032) are both completed.

## Context

The block explorer uses three Lambda functions for its compute layer:

1. **API Lambda (Rust/axum)**: Serves all public REST endpoints. Provisioned concurrency to minimize cold starts for user-facing requests.
2. **Ledger Processor Lambda**: Parses XDR files and writes explorer data. Triggered by S3 PutObject events. Auto-retried on failure with a DLQ for exhausted retries.
3. **Event Interpreter Lambda**: Enriches stored events with human-readable interpretations. Triggered by EventBridge (task 0037).

All three run on ARM64 (Graviton2) for cost efficiency and are VPC-attached in the private subnet.

### Source Code Location

- `infra/aws-cdk/lib/stacks/` (new `compute-stack.ts`)

## Implementation Plan

### Step 1: API Lambda Definition

Define the Rust API Lambda:

- Runtime: Rust (cargo-lambda) on ARM64/Graviton2 via `cargo-lambda-cdk` `RustFunction`
- Handler: Rust Lambda handler (axum via lambda_http)
- VPC: private subnet (from NetworkStack)
- Security group: Lambda SG (from NetworkStack)
- Provisioned concurrency: environment-specific (higher for production, lower for staging)
- Memory: 256 MB (sized for Rust binary overhead + query processing)
- Timeout: 30 seconds (appropriate for API response times)
- Environment variables: RDS Proxy endpoint, Secrets Manager ARN, environment name
- IAM execution role: RDS Proxy via Secrets Manager, CloudWatch Logs, X-Ray (defined in task 0040)

### Step 2: Ledger Processor Lambda Definition

Define the Ledger Processor Lambda:

- Runtime: Rust (cargo-lambda) on ARM64/Graviton2 via `cargo-lambda-cdk` `RustFunction`
- Trigger: S3 PutObject event (configured on stellar-ledger-data bucket from LedgerBucketStack)
- VPC: private subnet
- Security group: Lambda SG
- Memory: 512 MB (sized for XDR parsing + database writes)
- Timeout: 60 seconds (sufficient for <10s target latency with margin)
- Environment variables: RDS Proxy endpoint, Secrets Manager ARN, S3 bucket name
- IAM execution role: S3 GetObject on stellar-ledger-data, RDS Proxy, CloudWatch Logs, X-Ray (task 0040)
- Auto-retry: configured by S3 event notification (default 2 retries)
- DLQ: SQS queue for exhausted retries (see Step 4)

### Step 3: Event Interpreter Lambda Definition

Define the Event Interpreter Lambda:

- Runtime: Rust (cargo-lambda) on ARM64/Graviton2 via `cargo-lambda-cdk` `RustFunction`
- Trigger: EventBridge rate(5 minutes) (configured in task 0037)
- VPC: private subnet
- Security group: Lambda SG
- Memory: 256 MB (reads from DB, writes interpretations)
- Timeout: 300 seconds (sufficient for batch processing)
- Environment variables: RDS Proxy endpoint, Secrets Manager ARN
- IAM execution role: RDS Proxy, CloudWatch Logs, X-Ray (task 0040)

### Step 4: DLQ Configuration

Define the SQS Dead Letter Queue for the Ledger Processor:

- Receives S3 event records that exhausted Lambda retries
- Retention: 14 days (long enough for manual investigation)
- CloudWatch alarm on queue depth > 0 (indicates processing failures that need attention)
- Messages contain the original S3 event (bucket, key) for manual replay

### Step 5: EnvironmentConfig Extension

Add compute fields to `EnvironmentConfig` in `types.ts`:

- `apiLambdaMemory`, `apiLambdaTimeout`, `apiLambdaProvisionedConcurrency`
- `ledgerProcessorMemory`, `ledgerProcessorTimeout`
- `eventInterpreterMemory`, `eventInterpreterTimeout`

Update `envs/staging.json` and `envs/production.json` with appropriate values.

### Step 6: Stack Wiring

- Create `ComputeStack` in `stacks/compute-stack.ts`
- Wire in `app.ts`: pass `vpc`, `lambdaSg` from NetworkStack, `rdsProxyEndpoint` + `secretArn` from RdsStack, `bucket` from LedgerBucketStack
- Export API Lambda function ARN for task 0097 (API Gateway integration)

## Acceptance Criteria

- [ ] API Lambda is defined with Rust ARM64/Graviton2 (cargo-lambda-cdk RustFunction), provisioned concurrency, VPC attachment
- [ ] Ledger Processor Lambda is defined with S3 trigger, auto-retry, and SQS DLQ
- [ ] Event Interpreter Lambda is defined with appropriate timeout for batch processing
- [ ] All three Lambdas are VPC-attached in the private subnet
- [ ] SQS DLQ captures exhausted Ledger Processor retries with 14-day retention
- [ ] CloudWatch alarm on DLQ depth > 0
- [ ] All environment variables are parameterized, no hard-coded values
- [ ] All three Lambda functions configured with ARM64/Graviton2 runtime
- [ ] API Lambda has provisioned concurrency configured (environment-specific)
- [ ] No secret values embedded in Lambda deployment packages; secrets resolved at runtime via Secrets Manager
- [ ] Production Lambda database connection strings enforce TLS
- [ ] Failed XDR files remain in S3 after processing failure; DLQ messages contain bucket/key for manual replay
- [ ] Single Ledger Processor Lambda processes both live Galexie and historical backfill XDR files (no separate pipeline)
- [ ] EnvironmentConfig extended with compute fields, both env JSONs updated
- [ ] ComputeStack wired in app.ts with cross-stack references
- [ ] API Lambda ARN exported for API Gateway integration (task 0097)
- [ ] `cargo-lambda-cdk` added to package.json dependencies

## Notes

- Provisioned concurrency for the API Lambda eliminates cold starts but incurs cost even when idle. The staging environment should use a lower value.
- The DLQ is critical for operational visibility. A non-empty DLQ means ledgers are not being processed and requires investigation.
- Lambda ARM64/Graviton2 provides ~20% cost savings over x86_64 with comparable or better performance.
- The S3 event notification on the stellar-ledger-data bucket (LedgerBucketStack) must be configured to target the Ledger Processor Lambda ARN defined here.
- IAM execution roles (task 0040) and EventBridge rules (task 0037) are not yet implemented — Lambda definitions should be ready to receive them.
