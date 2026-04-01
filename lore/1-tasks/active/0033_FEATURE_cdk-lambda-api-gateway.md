---
id: '0033'
title: 'CDK: Lambda functions + SQS DLQ'
type: FEATURE
status: active
related_adr: ['0005']
related_tasks: ['0006', '0031', '0032', '0092', '0094', '0097']
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
  - date: 2026-04-01
    status: active
    who: fmazur
    note: 'Revised: removed provisioned concurrency (per 0092 findings), clarified S3→Lambda retry/DLQ mechanism, moved CloudWatch alarm to 0036, IAM via CDK grants'
  - date: 2026-04-01
    status: active
    who: fmazur
    note: 'Simplified to 2 Lambdas only (API + Indexer). Event Interpreter removed — not needed in milestone 1. Architecture: Galexie → S3 → Indexer → PostgreSQL ← API ← Frontend'
---

# CDK: Lambda functions + SQS DLQ

## Summary

Define two Lambda functions (API and Ledger Processor) and an SQS Dead Letter Queue using CDK. Both Lambdas run as Rust binaries on ARM64/Graviton2 via cargo-lambda-cdk RustFunction. The Ledger Processor is triggered by S3 PutObject (async invocation) with a DLQ for exhausted retries.

The simplified architecture:

```
Galexie → S3 → Indexer Lambda → PostgreSQL ← API Lambda ← Frontend
```

API Gateway is handled separately in task 0097.

## Status: Active

**Current state:** Not started. All dependencies resolved: VPC/networking (0031), storage (0032) completed, Cargo workspace (0094) merged.

## Context

The block explorer pipeline is intentionally simple:

1. **Galexie** (ECS Fargate) exports ledger data as XDR files to S3
2. **Ledger Processor Lambda** (Rust) parses XDR from S3, writes structured data to PostgreSQL
3. **API Lambda** (Rust/axum) serves REST API, reads from PostgreSQL
4. **Frontend** (React) consumes the API

Only two Lambda functions are needed. No Event Interpreter — enrichment can be added inline in the indexer if needed later.

### Source Code Location

- CDK: `infra/aws-cdk/src/lib/stacks/` (new `compute-stack.ts`)
- Rust binaries: `crates/api/` and `crates/indexer/` (from task 0094)

### Dependencies

- **Task 0094 (Cargo workspace)** — completed and merged. Provides `crates/api/` and `crates/indexer/`.
- **Task 0031 (VPC)** — completed. Provides `vpc`, `lambdaSecurityGroup`.
- **Task 0032 (RDS/S3)** — completed. Provides `dbProxy`, `dbSecret`, `bucket`.

## Implementation Plan

### Step 1: Add cargo-lambda-cdk dependency

Add `cargo-lambda-cdk` to `infra/aws-cdk/package.json`.

### Step 2: EnvironmentConfig extension

Add compute fields to `EnvironmentConfig` in `types.ts`:

- `apiLambdaMemory`, `apiLambdaTimeout`
- `ledgerProcessorMemory`, `ledgerProcessorTimeout`

Update `envs/staging.json` and `envs/production.json` with values:

- API: 256 MB, 30s
- Ledger Processor: 512 MB, 60s

### Step 3: Fix LedgerBucketStack wiring in app.ts

Currently `LedgerBucketStack` is instantiated without assigning to a variable — the bucket reference is not available for cross-stack use. Fix:

```ts
const ledgerBucket = new LedgerBucketStack(app, `${prefix}-LedgerBucket`, {
  env,
  config,
});
```

### Step 4: ComputeStack

Create `stacks/compute-stack.ts` with two Lambdas and a DLQ.

**API Lambda:**

- Construct: `RustFunction` with `manifestPath` = repo root, `binaryName: 'api'`
- Architecture: ARM64/Graviton2
- VPC: private subnet (from NetworkStack)
- Security group: Lambda SG (from NetworkStack)
- Memory: from config (default 256 MB)
- Timeout: from config (default 30s)
- Environment variables: `RDS_PROXY_ENDPOINT` (hostname), `SECRET_ARN` (Secrets Manager ARN), `ENV_NAME`
- IAM: `dbSecret.grantRead(apiLambda)`
- Tags: `Project`, `Environment`, `ManagedBy` (consistent with other stacks)

No provisioned concurrency — Rust cold starts are ~20-40ms (research 0092).

**Ledger Processor Lambda:**

- Construct: `RustFunction` with `manifestPath` = repo root, `binaryName: 'indexer'`
- Architecture: ARM64/Graviton2
- VPC: private subnet
- Security group: Lambda SG
- Memory: from config (default 512 MB)
- Timeout: from config (default 60s)
- Environment variables: `RDS_PROXY_ENDPOINT`, `SECRET_ARN`, `BUCKET_NAME`, `ENV_NAME`
- S3 trigger: `bucket.addEventNotification(OBJECT_CREATED, new LambdaDestination(processorLambda))`
- Async invocation config via `EventInvokeConfig`: `retryAttempts: 2`, `onFailure: new SqsDestination(dlq)`
- IAM: `bucket.grantRead(processorLambda)`, `dbSecret.grantRead(processorLambda)`

**SQS DLQ:**

- Retention: 14 days
- Receives failed async invocations after retries exhausted
- Messages contain original S3 event (bucket, key) for manual replay
- CloudWatch alarm deferred to task 0036

### Step 5: Stack wiring in app.ts

- Wire ComputeStack: `vpc`, `lambdaSg` from NetworkStack; `dbProxy`, `dbSecret` from RdsStack; `bucket` from LedgerBucketStack
- CfnOutputs: `ApiLambdaArn`, `ProcessorLambdaArn`, `DlqUrl`
- Export API Lambda function for task 0097 (API Gateway integration)

## Acceptance Criteria

- [ ] API Lambda defined with Rust ARM64/Graviton2 (cargo-lambda-cdk RustFunction), VPC attachment
- [ ] Ledger Processor Lambda defined with S3 trigger, maxRetryAttempts: 2, onFailure → SQS DLQ
- [ ] Both Lambdas VPC-attached in private subnet with Lambda SG
- [ ] SQS DLQ created with 14-day retention
- [ ] All environment variables parameterized, no hard-coded values
- [ ] Both Lambda functions configured with ARM64/Graviton2
- [ ] No secret values in Lambda packages; secrets resolved at runtime via Secrets Manager
- [ ] Production database connections enforce TLS (via RDS Proxy `requireTLS: true`)
- [ ] Failed XDR files remain in S3; DLQ messages contain bucket/key for replay
- [ ] Single Ledger Processor Lambda processes both live Galexie and historical backfill XDR files
- [ ] EnvironmentConfig extended with compute fields, both env JSONs updated
- [ ] ComputeStack wired in app.ts with cross-stack references
- [ ] LedgerBucketStack bucket reference passed to ComputeStack (fix existing wiring)
- [ ] API Lambda ARN exported for API Gateway integration (task 0097)
- [ ] CfnOutputs for ApiLambdaArn, ProcessorLambdaArn, DlqUrl
- [ ] `cargo-lambda-cdk` added to package.json dependencies
- [ ] IAM via CDK `grant*()` methods (auto-generated execution roles)
- [ ] Tags (Project, Environment, ManagedBy) consistent with other stacks

## Notes

- **No provisioned concurrency.** Research 0092 measured Rust ARM64 cold starts at ~20-40ms. Well below any user-facing threshold.
- **No Event Interpreter Lambda.** Simplified architecture: Galexie → S3 → Indexer → DB ← API ← Frontend. If enrichment is needed later (milestone 2), it can be added inline in the indexer or as a separate Lambda then.
- The DLQ is critical for operational visibility. A non-empty DLQ means ledgers are not being processed. CloudWatch alarm in task 0036.
- Lambda ARM64/Graviton2 provides ~20% cost savings over x86_64.
- `RustFunction` uses `manifestPath` pointing to repo root (where `Cargo.toml` workspace lives) and `binaryName` to select the crate.
- Lambda env vars use `RDS_PROXY_ENDPOINT` (hostname only) + `SECRET_ARN` — not a full `DATABASE_URL` connection string. Lambda resolves credentials from Secrets Manager at runtime. This is the standard pattern with RDS Proxy + Secrets Manager.
