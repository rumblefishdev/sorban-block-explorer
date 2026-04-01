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
---

# CDK: Lambda functions + SQS DLQ

## Summary

Define the three Lambda functions (API, Ledger Processor, Event Interpreter) and the SQS Dead Letter Queue using CDK. All Lambdas run as Rust binaries on ARM64/Graviton2 via cargo-lambda-cdk RustFunction. The Ledger Processor is triggered by S3 PutObject (async invocation) with a DLQ for exhausted retries. The Event Interpreter is triggered by EventBridge (configured in task 0037).

API Gateway, WAF attachment, and usage plans are handled separately in task 0097.

## Status: Active

**Current state:** Not started. Dependencies on VPC/networking (task 0031) and storage (task 0032) are both completed. Cargo workspace (task 0094) is being developed in parallel by another team member — use `RustFunction` with path to workspace once 0094 merges; until then `cdk synth` will fail on compute stack.

## Context

The block explorer uses three Lambda functions for its compute layer:

1. **API Lambda (Rust/axum)**: Serves all public REST endpoints. Rust cold starts are ~20-40ms (per research 0092), so provisioned concurrency is not needed.
2. **Ledger Processor Lambda**: Parses XDR files and writes explorer data. Triggered by S3 PutObject events (async invocation). Auto-retried by Lambda service (max 2 retries) with DLQ for exhausted retries.
3. **Event Interpreter Lambda**: Enriches stored events with human-readable interpretations. Triggered by EventBridge (task 0037).

All three run on ARM64 (Graviton2) for cost efficiency and are VPC-attached in the private subnet.

### Source Code Location

- `infra/aws-cdk/lib/stacks/` (new `compute-stack.ts`)

### Dependencies

- **Task 0094 (Cargo workspace)** — `RustFunction` needs Rust source code at `rust/crates/`. Being developed in parallel. `cdk synth` for ComputeStack requires this to be present.
- **Task 0031 (VPC)** — completed. Provides `vpc`, `lambdaSecurityGroup`.
- **Task 0032 (RDS/S3)** — completed. Provides `dbProxy`, `dbSecret`, `bucket`.

## Implementation Plan

### Step 1: Fix LedgerBucketStack wiring in app.ts

Currently `LedgerBucketStack` is instantiated without assigning to a variable — the bucket reference is not available for cross-stack use. Fix:

```ts
const ledgerBucket = new LedgerBucketStack(app, `${prefix}-LedgerBucket`, {
  env,
  config,
});
```

### Step 2: API Lambda Definition

Define the Rust API Lambda:

- Construct: `cargo-lambda-cdk` `RustFunction` pointing to `rust/crates/api`
- Architecture: ARM64/Graviton2
- VPC: private subnet (from NetworkStack)
- Security group: Lambda SG (from NetworkStack)
- Memory: 256 MB
- Timeout: 30 seconds
- Environment variables: RDS Proxy endpoint, Secrets Manager ARN, environment name
- IAM: CDK auto-generated role + `dbSecret.grantRead()`, `dbProxy.grantConnect()`

No provisioned concurrency — Rust cold starts are ~20-40ms (research 0092).

### Step 3: Ledger Processor Lambda Definition

Define the Ledger Processor Lambda:

- Construct: `cargo-lambda-cdk` `RustFunction` pointing to `rust/crates/indexer`
- Architecture: ARM64/Graviton2
- VPC: private subnet
- Security group: Lambda SG
- Memory: 512 MB (XDR parsing + database writes)
- Timeout: 60 seconds
- Environment variables: RDS Proxy endpoint, Secrets Manager ARN, S3 bucket name
- Trigger: S3 PutObject event on stellar-ledger-data bucket (add `s3.EventType.OBJECT_CREATED` notification from LedgerBucketStack)
- Async invocation config: `maxRetryAttempts: 2`
- On failure destination: SQS DLQ (see Step 5)
- IAM: `bucket.grantRead()`, `dbSecret.grantRead()`, `dbProxy.grantConnect()`

### Step 4: Event Interpreter Lambda Definition

Define the Event Interpreter Lambda:

- Construct: `cargo-lambda-cdk` `RustFunction` pointing to `rust/crates/interpreter` (or shared binary with mode flag — TBD with task 0094)
- Architecture: ARM64/Graviton2
- VPC: private subnet
- Security group: Lambda SG
- Memory: 256 MB
- Timeout: 300 seconds (batch processing)
- Environment variables: RDS Proxy endpoint, Secrets Manager ARN
- Trigger: EventBridge rate(5 minutes) — configured in task 0037, Lambda just needs to exist
- IAM: `dbSecret.grantRead()`, `dbProxy.grantConnect()`

### Step 5: SQS DLQ

Define the SQS Dead Letter Queue for the Ledger Processor:

- Receives async invocation failures after retries exhausted
- Retention: 14 days
- Messages contain the original S3 event (bucket, key) for manual replay
- CloudWatch alarm on queue depth > 0 deferred to task 0036

### Step 6: EnvironmentConfig Extension

Add compute fields to `EnvironmentConfig` in `types.ts`:

- `apiLambdaMemory`, `apiLambdaTimeout`
- `ledgerProcessorMemory`, `ledgerProcessorTimeout`
- `eventInterpreterMemory`, `eventInterpreterTimeout`

Update `envs/staging.json` and `envs/production.json` with appropriate values.

### Step 7: Stack Wiring

- Create `ComputeStack` in `stacks/compute-stack.ts`
- Wire in `app.ts`: pass `vpc`, `lambdaSg` from NetworkStack, `rdsProxy` + `dbSecret` from RdsStack, `bucket` from LedgerBucketStack
- Export API Lambda function ARN for task 0097 (API Gateway integration)
- Add `cargo-lambda-cdk` to `package.json` dependencies

## Acceptance Criteria

- [ ] API Lambda defined with Rust ARM64/Graviton2 (cargo-lambda-cdk RustFunction), VPC attachment
- [ ] Ledger Processor Lambda defined with S3 async invocation trigger, maxRetryAttempts: 2, onFailure → SQS DLQ
- [ ] Event Interpreter Lambda defined with appropriate timeout for batch processing
- [ ] All three Lambdas VPC-attached in private subnet with Lambda SG
- [ ] SQS DLQ created with 14-day retention for Ledger Processor failures
- [ ] All environment variables parameterized, no hard-coded values
- [ ] All three Lambda functions configured with ARM64/Graviton2
- [ ] No secret values embedded in Lambda packages; secrets resolved at runtime via Secrets Manager
- [ ] Production Lambda database connection strings enforce TLS (via RDS Proxy `requireTLS: true`)
- [ ] Failed XDR files remain in S3 after processing failure; DLQ messages contain bucket/key for manual replay
- [ ] Single Ledger Processor Lambda processes both live Galexie and historical backfill XDR files
- [ ] EnvironmentConfig extended with compute fields, both env JSONs updated
- [ ] ComputeStack wired in app.ts with cross-stack references
- [ ] LedgerBucketStack bucket reference passed to ComputeStack (fix existing wiring)
- [ ] API Lambda ARN exported for API Gateway integration (task 0097)
- [ ] `cargo-lambda-cdk` added to package.json dependencies
- [ ] IAM via CDK `grant*()` methods (auto-generated execution roles)

## Notes

- **No provisioned concurrency.** Research 0092 measured Rust ARM64 cold starts at ~20-40ms for our expected binary size. Well below any user-facing threshold. Revisit if monitoring shows otherwise.
- The DLQ is critical for operational visibility. A non-empty DLQ means ledgers are not being processed and requires investigation. CloudWatch alarm for DLQ depth is in task 0036.
- Lambda ARM64/Graviton2 provides ~20% cost savings over x86_64 with comparable or better performance.
- The S3 event notification on the stellar-ledger-data bucket must be added in ComputeStack (bucket imported via cross-stack ref).
- EventBridge rules (task 0037) are not yet implemented — Event Interpreter Lambda just needs to exist and be referenceable.
- **Cargo workspace dependency:** `RustFunction` requires Rust source at build time. Task 0094 (being developed in parallel) will provide `rust/crates/`. Until it merges, `cdk synth` on ComputeStack will fail — this is expected.
