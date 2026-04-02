---
id: '0099'
title: 'CDK: ComputeStack with 2 Rust Lambdas + SQS DLQ'
type: FEATURE
status: completed
related_adr: ['0005']
related_tasks: ['0031', '0032', '0033', '0094', '0097']
tags: [priority-high, effort-medium, layer-infra]
milestone: 1
links: []
history:
  - date: 2026-04-01
    status: backlog
    who: fmazur
    note: 'Task created — implements CDK infrastructure for task 0033 scope'
  - date: 2026-04-01
    status: active
    who: fmazur
    note: 'Started implementation'
  - date: 2026-04-01
    status: completed
    who: fmazur
    note: >
      Implemented ComputeStack with 2 Rust Lambdas (API + Indexer), SQS DLQ,
      S3 trigger with .xdr.zst suffix filter, EventInvokeConfig, IAM grants,
      explicit LogGroups, tags, CfnOutputs. 9 files changed.
      All acceptance criteria met. Build + lint + typecheck + all tests passing.
---

# CDK: ComputeStack with 2 Rust Lambdas + SQS DLQ

## Summary

Implement the CDK ComputeStack defining two Rust Lambda functions (API and Ledger Processor) and an SQS Dead Letter Queue. Both Lambdas built via cargo-lambda-cdk RustFunction on ARM64/Graviton2. The Ledger Processor is triggered by S3 PutObject (filtered to .xdr.zst suffix) with async retry and DLQ for exhausted retries.

## Status: Completed

## Context

Task 0033 defined the requirements for the compute layer. This task implements it as CDK infrastructure. The simplified architecture:

```
Galexie → S3 → Indexer Lambda → PostgreSQL ← API Lambda ← Frontend
```

Dependencies all resolved: NetworkStack (0031), RdsStack/LedgerBucketStack (0032), Cargo workspace (0094).

## Acceptance Criteria

- [x] API Lambda defined with RustFunction ARM64/Graviton2, VPC private subnet
- [x] Ledger Processor Lambda defined with S3 trigger (.xdr.zst suffix filter), retryAttempts: 2, onFailure → SQS DLQ
- [x] SQS DLQ with 14-day retention
- [x] EnvironmentConfig extended with compute fields (apiLambdaMemory/Timeout, indexerLambdaMemory/Timeout)
- [x] Both env JSONs updated (staging + production)
- [x] ComputeStack wired in app.ts with cross-stack references
- [x] LedgerBucketStack bucket reference fix (assigned to variable)
- [x] Cross-stack cyclic dependency resolved (bucket imported via fromBucketAttributes)
- [x] cargo-lambda-cdk ^0.0.36 added to package.json
- [x] IAM via grant\*() methods (dbSecret.grantRead, bucket.grantRead)
- [x] Explicit function names (${envName}-soroban-explorer-api/indexer)
- [x] Explicit LogGroups with /aws/lambda/ convention, 30-day retention, DESTROY removal policy
- [x] Tags (Project, Environment, ManagedBy) consistent with other stacks
- [x] CfnOutputs (ApiLambdaArn, ProcessorLambdaArn, DlqUrl)
- [x] cargoWorkspacePath injected from entry points via CreateAppOptions
- [x] ComputeStack + ComputeStackProps exported in index.ts

## Implementation Notes

### Files changed (9)

| File                                            | Change                                             |
| ----------------------------------------------- | -------------------------------------------------- |
| `infra/aws-cdk/package.json`                    | Added `cargo-lambda-cdk: ^0.0.36`                  |
| `infra/aws-cdk/src/lib/types.ts`                | Extended EnvironmentConfig with 4 compute fields   |
| `infra/aws-cdk/envs/staging.json`               | Added compute values (256/30, 512/60)              |
| `infra/aws-cdk/envs/production.json`            | Added compute values (256/30, 512/60)              |
| `infra/aws-cdk/src/lib/stacks/compute-stack.ts` | **New** — ComputeStack                             |
| `infra/aws-cdk/src/lib/app.ts`                  | Fixed wiring, added ComputeStack, CreateAppOptions |
| `infra/aws-cdk/src/index.ts`                    | Export ComputeStack + ComputeStackProps            |
| `infra/aws-cdk/src/bin/staging.ts`              | Compute repoRoot, pass cargoWorkspacePath          |
| `infra/aws-cdk/src/bin/production.ts`           | Compute repoRoot, pass cargoWorkspacePath          |

## Design Decisions

### From Plan

1. **Two Lambdas, not three**: Event Interpreter removed — milestone 2. Architecture: Galexie → S3 → Indexer → DB ← API ← Frontend.
2. **No provisioned concurrency**: Rust ARM64 cold starts ~20-40ms (research 0092).
3. **RDS_PROXY_ENDPOINT + SECRET_ARN, not DATABASE_URL**: Runtime credential resolution from Secrets Manager.
4. **Async invocation retries + DLQ**: EventInvokeConfig simpler than SQS-based pipeline for this use case.

### Emerged

5. **cargoWorkspacePath as dependency injection**: Replaced fragile `path.join(__dirname, '../../../../..')` with prop injection from entry points via `import.meta.url`. Single source of truth.
6. **CreateAppOptions interface**: Breaking API change to `createApp()` — object param instead of bare config. Required for cargoWorkspacePath without polluting EnvironmentConfig.
7. **Explicit functionName**: From .temp poglądowa infra pattern. Prevents random CDK-generated names.
8. **Explicit LogGroup instead of logRetention**: `logRetention` is deprecated in aws-cdk-lib. Created per-Lambda LogGroups with `/aws/lambda/` naming convention and DESTROY removal policy.
9. **S3 suffix filter `.xdr.zst`**: Prevents triggering on non-ledger objects. From PR review feedback.
10. **Config naming `indexerLambda*` not `ledgerProcessor*`**: Aligned with crate name (`crates/indexer/`).
11. **Bucket imported via `fromBucketAttributes`**: Direct `IBucket` cross-stack ref caused cyclic dependency (LedgerBucketStack ↔ ComputeStack). Resolved by passing bucket ARN/name as strings and importing via `Bucket.fromBucketAttributes()` — same pattern as `Vpc.fromLookup` in poglądowa infra. Breaks the CDK token chain while keeping full bucket API access.

## Issues Encountered

- **`logRetention` deprecated**: CDK warning on `cdk diff`. Fixed by replacing with explicit `logs.LogGroup`.
- **`cargo-lambda` not installed**: `cdk synth` fails without it — RustFunction needs it to build binaries. Resolved by `pip3 install cargo-lambda`.
- **Pre-commit/push hook failures**: Cargo not in PATH for hook subshell. Resolved by sourcing `$HOME/.cargo/env` before git commands.
- **Cross-stack cyclic dependency**: `bucket.addEventNotification()` on a cross-stack `IBucket` caused CDK to create a custom resource in LedgerBucketStack referencing Lambda ARN from ComputeStack, while ComputeStack referenced bucket ARN from LedgerBucketStack — cycle. Resolved by passing bucket ARN/name as strings and using `Bucket.fromBucketAttributes()` to import the bucket locally in ComputeStack.
