---
id: '0033'
title: 'CDK: Lambda functions + SQS DLQ'
type: FEATURE
status: completed
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
  - date: 2026-04-01
    status: completed
    who: fmazur
    note: >
      Implemented ComputeStack with 2 Rust Lambdas (API + Indexer), SQS DLQ,
      S3 trigger, EventInvokeConfig, IAM grants, tags, CfnOutputs.
      7 files changed. All 18 acceptance criteria met. Build + lint + typecheck passing.
---

# CDK: Lambda functions + SQS DLQ

## Summary

Define two Lambda functions (API and Ledger Processor) and an SQS Dead Letter Queue using CDK. Both Lambdas run as Rust binaries on ARM64/Graviton2 via cargo-lambda-cdk RustFunction. The Ledger Processor is triggered by S3 PutObject (async invocation) with a DLQ for exhausted retries.

The simplified architecture:

```
Galexie → S3 → Indexer Lambda → PostgreSQL ← API Lambda ← Frontend
```

API Gateway is handled separately in task 0097.

## Status: Completed

All 18 acceptance criteria met. Build, lint, and typecheck passing.

## Context

The block explorer pipeline is intentionally simple:

1. **Galexie** (ECS Fargate) exports ledger data as XDR files to S3
2. **Ledger Processor Lambda** (Rust) parses XDR from S3, writes structured data to PostgreSQL
3. **API Lambda** (Rust/axum) serves REST API, reads from PostgreSQL
4. **Frontend** (React) consumes the API

Only two Lambda functions are needed. No Event Interpreter — enrichment can be added inline in the indexer if needed later.

### Source Code Location

- CDK: `infra/aws-cdk/src/lib/stacks/compute-stack.ts`
- Rust binaries: `crates/api/` and `crates/indexer/` (from task 0094)

### Dependencies

- **Task 0094 (Cargo workspace)** — completed and merged. Provides `crates/api/` and `crates/indexer/`.
- **Task 0031 (VPC)** — completed. Provides `vpc`, `lambdaSecurityGroup`.
- **Task 0032 (RDS/S3)** — completed. Provides `dbProxy`, `dbSecret`, `bucket`.

## Acceptance Criteria

- [x] API Lambda defined with Rust ARM64/Graviton2 (cargo-lambda-cdk RustFunction), VPC attachment
- [x] Ledger Processor Lambda defined with S3 trigger, maxRetryAttempts: 2, onFailure → SQS DLQ
- [x] Both Lambdas VPC-attached in private subnet with Lambda SG
- [x] SQS DLQ created with 14-day retention
- [x] All environment variables parameterized, no hard-coded values
- [x] Both Lambda functions configured with ARM64/Graviton2
- [x] No secret values in Lambda packages; secrets resolved at runtime via Secrets Manager
- [x] Production database connections enforce TLS (via RDS Proxy `requireTLS: true`)
- [x] Failed XDR files remain in S3; DLQ messages contain bucket/key for replay
- [x] Single Ledger Processor Lambda processes both live Galexie and historical backfill XDR files
- [x] EnvironmentConfig extended with compute fields, both env JSONs updated
- [x] ComputeStack wired in app.ts with cross-stack references
- [x] LedgerBucketStack bucket reference passed to ComputeStack (fix existing wiring)
- [x] API Lambda ARN exported for API Gateway integration (task 0097)
- [x] CfnOutputs for ApiLambdaArn, ProcessorLambdaArn, DlqUrl
- [x] `cargo-lambda-cdk` added to package.json dependencies
- [x] IAM via CDK `grant*()` methods (auto-generated execution roles)
- [x] Tags (Project, Environment, ManagedBy) consistent with other stacks

## Implementation Notes

### Files changed (7)

| File                                            | Change                                                          |
| ----------------------------------------------- | --------------------------------------------------------------- |
| `infra/aws-cdk/package.json`                    | Added `cargo-lambda-cdk: ^0.0.36`                               |
| `infra/aws-cdk/src/lib/types.ts`                | Extended EnvironmentConfig with 4 compute fields                |
| `infra/aws-cdk/envs/staging.json`               | Added compute values (256/30, 512/60)                           |
| `infra/aws-cdk/envs/production.json`            | Added compute values (256/30, 512/60)                           |
| `infra/aws-cdk/src/lib/stacks/compute-stack.ts` | **New** — ComputeStack with 2 Lambdas + DLQ                     |
| `infra/aws-cdk/src/lib/app.ts`                  | Fixed wiring, added ComputeStack, changed to `CreateAppOptions` |
| `infra/aws-cdk/src/index.ts`                    | Export ComputeStack + ComputeStackProps                         |

Additionally modified entry points (`bin/staging.ts`, `bin/production.ts`) to compute and pass `cargoWorkspacePath`.

### ComputeStack resources

- **API Lambda** (`staging-soroban-explorer-api`): RustFunction, ARM64, 256MB/30s, VPC private subnet
- **Processor Lambda** (`staging-soroban-explorer-indexer`): RustFunction, ARM64, 512MB/60s, VPC private subnet, S3 OBJECT_CREATED trigger
- **SQS DLQ** (`staging-ledger-processor-dlq`): 14-day retention, receives failed async invocations
- **EventInvokeConfig**: retryAttempts: 2, onFailure → DLQ
- **IAM grants**: dbSecret.grantRead (both), bucket.grantRead (processor only)
- **CloudWatch log retention**: 30 days (both Lambdas)

## Design Decisions

### From Plan

1. **Two Lambdas, not three**: Event Interpreter removed — enrichment deferred to milestone 2. Architecture simplified to Galexie → S3 → Indexer → DB ← API ← Frontend.

2. **No provisioned concurrency**: Research 0092 measured Rust ARM64 cold starts at ~20-40ms. No need for provisioned concurrency.

3. **RDS_PROXY_ENDPOINT + SECRET_ARN, not DATABASE_URL**: Lambda resolves credentials from Secrets Manager at runtime. No full connection string in env vars.

4. **Async invocation retries + DLQ, not SQS-based pipeline**: S3 → Lambda async invocation with EventInvokeConfig is simpler than S3 → SQS → Lambda for this use case. DLQ captures failures after retry exhaustion.

5. **Memory/timeout in EnvironmentConfig**: Follows YAGNI pattern — each stack adds only the config fields it consumes. Values configurable per environment.

### Emerged

6. **`cargoWorkspacePath` as dependency injection**: Original plan had `manifestPath` computed via `path.join(__dirname, '../../../../..')` (5 levels of `..`). This is fragile — breaks if outDir changes or file moves. Changed to pass `cargoWorkspacePath` from entry points (`bin/staging.ts`, `bin/production.ts`) via `CreateAppOptions` interface. Entry points know their location via `import.meta.url`.

7. **`createApp(config)` → `createApp({ config, cargoWorkspacePath })`**: Breaking API change to `createApp` — now accepts `CreateAppOptions` object instead of bare `EnvironmentConfig`. Required to pass workspace path without polluting `EnvironmentConfig` (which is env-specific data, not build paths).

8. **Explicit `functionName` per Lambda**: Poglądowa infra (`.temp/`) sets explicit function names. Without it, CDK generates random names like `Explorer-staging-Compute-ApiFunction8A2BC-xyz`. Added `${envName}-soroban-explorer-api` and `${envName}-soroban-explorer-indexer` for discoverability in AWS console and CloudWatch.

9. **`logRetention: ONE_MONTH`**: Not in original plan. CloudWatch Logs default to infinite retention, generating unbounded cost. Set 30-day retention on both Lambdas.

10. **Config field naming: `indexerLambdaMemory` not `ledgerProcessorMemory`**: Plan said `ledgerProcessorMemory/Timeout` but the crate is named `indexer` (`binaryName: 'indexer'`). Aligned config naming with crate name for consistency.

## Issues Encountered

- **`LOG_RETENTION_DAYS` unused variable**: Initially declared `const LOG_RETENTION_DAYS = 30` but `logRetention` uses the `logs.RetentionDays.ONE_MONTH` enum, not a number. TypeScript flagged it as unused. Removed the constant.

- **`manifestPath` fragility**: The 5-level `path.join(__dirname, '../../../../..')` approach counted directory levels from compiled output (`dist/lib/stacks/`). Any change to `outDir`, `rootDir`, or file location would silently break. Resolved by injecting the path from entry points.

## Notes

- **No provisioned concurrency.** Research 0092 measured Rust ARM64 cold starts at ~20-40ms. Well below any user-facing threshold.
- **No Event Interpreter Lambda.** Simplified architecture: Galexie → S3 → Indexer → DB ← API ← Frontend. If enrichment is needed later (milestone 2), it can be added inline in the indexer or as a separate Lambda then.
- The DLQ is critical for operational visibility. A non-empty DLQ means ledgers are not being processed. CloudWatch alarm in task 0036.
- Lambda ARM64/Graviton2 provides ~20% cost savings over x86_64.
- `RustFunction` uses `manifestPath` pointing to repo root (where `Cargo.toml` workspace lives) and `binaryName` to select the crate.
- Lambda env vars use `RDS_PROXY_ENDPOINT` (hostname only) + `SECRET_ARN` — not a full `DATABASE_URL` connection string. Lambda resolves credentials from Secrets Manager at runtime.
- `cdk synth` requires Rust toolchain + cargo-lambda installed. Without it, synth fails — this is expected. TypeScript build (`nx build`) works without Rust.
