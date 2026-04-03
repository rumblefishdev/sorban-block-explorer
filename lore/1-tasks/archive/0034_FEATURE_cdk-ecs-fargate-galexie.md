---
id: '0034'
title: 'CDK: ECS Fargate for Galexie live + backfill'
type: FEATURE
status: completed
related_adr: []
related_tasks: ['0006', '0031', '0001', '0040']
tags: [priority-medium, effort-medium, layer-infra]
milestone: 1
links:
  - docs/architecture/infrastructure/infrastructure-overview.md
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
  - date: 2026-04-03
    status: active
    who: fmazur
    note: 'Activated task'
  - date: 2026-04-03
    status: active
    who: fmazur
    note: >
      Review: merged ECS-relevant scope from task 0040 (IAM task roles,
      ECR repository, S3 VPC endpoint policy). Corrected S3 key format,
      Soroban activation ledger, added Fargate sizing, operational
      concerns (circuit breaker, ECS Exec, graceful shutdown, logging).
      Added EnvironmentConfig fields for ECS/ingestion.
  - date: 2026-04-03
    status: completed
    who: fmazur
    note: >
      Implemented IngestionStack with 8 steps. 1 new file
      (ingestion-stack.ts, 310 lines), 7 modified files. Key
      resources: ECR repo, ECS cluster, Galexie live service,
      backfill task def, IAM roles, VPC endpoint policy, log groups.
      7 new EnvironmentConfig fields. Multiple review rounds caught:
      VPC endpoint policy too restrictive, missing volume mounts for
      readonlyRootFilesystem, Container Insights Enhanced→Enabled,
      enableExecuteCommand hardcoded→config-driven.
---

# CDK: ECS Fargate for Galexie live + backfill

## Summary

New `IngestionStack` with ECS Fargate infrastructure for two workloads: (1) a continuous Galexie service for live ledger export, and (2) an on-demand backfill Fargate task for historical data. Includes ECS cluster, task definitions, IAM roles (task + execution), ECR repository, CloudWatch Logs, and S3 VPC endpoint policy refinement.

## Acceptance Criteria

**ECS Core:**

- [x] New `IngestionStack` with dependency on `NetworkStack` and `LedgerBucketStack`
- [x] ECS cluster defined with Container Insights enabled
- [x] Galexie continuous service runs as ECS Fargate with desired count 1
- [x] Deployment circuit breaker enabled with automatic rollback
- [x] `enableExecuteCommand` config-driven (`ecsExecEnabled` in env JSON)
- [x] `stopTimeout: 120` for graceful Galexie shutdown

**Fargate Sizing (environment-driven):**

- [x] CPU, memory, ephemeral storage read from EnvironmentConfig
- [x] Staging: 2048 cpu / 8192 MiB / 30 GiB ephemeral
- [x] Production: 4096 cpu / 16384 MiB / 30 GiB ephemeral

**Service Configuration:**

- [x] Captive Core network passphrase is environment-driven (mainnet for prod, testnet for staging)
- [x] S3 destination uses bucket name from `LedgerBucketStack`
- [x] Galexie is checkpoint-aware (append mode, auto-resumes from last S3 object)
- [x] Container health check via process liveness (not S3 cadence)

**Backfill:**

- [x] Separate task definition for backfill workload
- [x] Backfill accepts configurable start/end ledger range via env var overrides
- [x] Multiple parallel backfill tasks with non-overlapping ranges are supported
- [x] Backfill uses BoundedRange mode (HTTPS archives only, no peer connections)

**IAM & ECR (merged from task 0040):**

- [x] ECS task role: `s3:PutObject` + `s3:ListBucket` on stellar-ledger-data bucket, CloudWatch Logs — no RDS
- [x] ECS task execution role: ECR pull, CloudWatch Logs
- [x] CDK L2 grants used; ListBucket via inline policy (IBucket limitation on imported buckets)
- [x] ECR repository with lifecycle policy (retain 10 images, expire untagged after 7 days)
- [x] ECR image scanning enabled (`imageScanOnPush: true`)

**Networking & Security:**

- [x] Both workloads run in private subnets with `ecsSecurityGroup`
- [x] S3 access routes through VPC endpoint, not NAT Gateway
- [x] Stellar network/archive access routes through NAT Gateway
- [x] S3 VPC endpoint policy restricts access to project buckets only
- [x] No secret values baked into container image
- [x] `readonlyRootFilesystem: true` with ephemeral storage volume mounts (`/data`, `/tmp`)

**Logging:**

- [x] `awslogs` driver with pre-created log groups
- [x] Log retention: 30 days (staging), 90 days (production) — from EnvironmentConfig

## Implementation Notes

**New file:** `infra/src/lib/stacks/ingestion-stack.ts` (~310 lines)

**Modified files:**

- `infra/src/lib/app.ts` — wired IngestionStack with deps on Network + LedgerBucket
- `infra/src/lib/stacks/network-stack.ts` — S3 VPC endpoint policy (bucket-scoped + CDK buckets)
- `infra/src/lib/types.ts` — 8 new EnvironmentConfig fields
- `infra/src/index.ts` — exported IngestionStack
- `infra/envs/staging.json` — ECS config values (testnet)
- `infra/envs/production.json` — ECS config values (mainnet)
- `infra/Makefile` — deploy-staging-ingestion, deploy-production-ingestion targets

**EnvironmentConfig fields added:**
`galexieCpu`, `galexieMemory`, `galexieEphemeralStorage`, `galexieDesiredCount`, `stellarNetworkPassphrase`, `ecsLogRetentionDays`, `galexieStopTimeout`, `ecsExecEnabled`

## Issues Encountered

- **VPC endpoint policy too restrictive:** Initially set `s3:GetObject`, `s3:PutObject`, `s3:ListBucket` — but `grantWrite()` also needs `s3:AbortMultipartUpload` and `s3:DeleteObject`, and `grantRead()` needs `s3:GetBucketLocation`. VPC endpoint policy is AND with IAM, so missing actions would cause runtime failures. Fix: changed to `s3:*` on our bucket (bucket-level restriction), IAM controls actions.

- **Missing volume mounts with readonlyRootFilesystem:** Set `readonlyRootFilesystem: true` but forgot to add writable volume mounts. Galexie/Captive Core need `/data` (ledger state) and `/tmp` (temp files). Without mounts, container would crash on start. Fix: added ephemeral volumes with mount points.

- **ECR lifecycle rule priority:** CDK requires `TagStatus.ANY` rules to have the highest (last) priority number. Initially had ANY at priority 1, UNTAGGED at 2 — reversed to fix.

- **`containerInsights` deprecated:** CDK warns about `containerInsights: true`. Replaced with `containerInsightsV2: ecs.ContainerInsights.ENABLED`.

- **`IBucket.grant()` not available on imported buckets:** `s3.Bucket.fromBucketAttributes()` returns `IBucket` which doesn't expose `.grant()`. Used `addToPrincipalPolicy()` with inline `PolicyStatement` for `s3:ListBucket`.

## Design Decisions

### From Plan

1. **Separate IngestionStack:** Not extending ComputeStack. Clean separation between Lambda compute and ECS ingestion, consistent with architecture overview and README.

2. **Environment-driven Fargate sizing:** All sizing in `EnvironmentConfig` + env JSON files. No hardcoded values in stack code.

3. **S3 VPC endpoint policy:** Bucket-level restriction (which buckets can be accessed), IAM provides action-level control. Standard defense-in-depth pattern.

4. **CDK L2 grants:** `bucket.grantWrite()`, `repository.grantPull()` instead of custom inline policies. Consistent with ComputeStack pattern.

### Emerged

5. **`ecsExecEnabled` config flag:** Original plan had `enableExecuteCommand: true` unconditionally. Security review flagged this for production — SSM permissions with `resources: '*'` widen attack surface. Changed to config-driven flag: staging=true, production=false. Enables prod debugging by changing JSON + deploy.

6. **Container Insights ENABLED not ENHANCED:** Original implementation used ENHANCED (full observability). Changed to ENABLED (basic metrics) — ENHANCED adds GPU monitoring and detailed network metrics unnecessary for 1 container. Saves ~$5/mo.

7. **30 GiB ephemeral storage (not 100 GiB):** Research task 0001 recommended 100 GiB minimum. Senior feedback: Galexie uploads to S3 and deletes local files — no accumulation. 30 GiB covers Captive Core catchup state + current batch buffer. Saves ~$5/mo.

8. **Volume mounts for readonlyRootFilesystem:** Plan mentioned readonlyRootFilesystem but not explicit mount points. Added `/data` (Captive Core state) and `/tmp` (general temp) as writable volumes on ephemeral storage.

9. **`s3:*` on VPC endpoint policy:** Plan said restrict to specific actions. Changed to `s3:*` after discovering CDK L2 grants include more actions than initially listed (AbortMultipartUpload, DeleteObject, GetBucketLocation). Bucket-level restriction is the correct layer for endpoint policies.

## Future Work

- Galexie container image build and ECR push (task 0039 — CI/CD pipeline)
- CloudWatch alarms for S3 cadence monitoring, ingestion lag (task 0036)
- Task 0040 retains: GitHub Actions OIDC, deploy roles (reduced scope)
