---
title: 'Synthesis: CDK architecture decisions for the explorer'
type: synthesis
status: developing
spawned_from: null
spawns: []
tags: [cdk, architecture, decisions]
links: []
history:
  - date: 2026-03-26
    status: developing
    who: stkrolikiewicz
    note: 'Synthesis of CDK research findings'
---

# Synthesis: CDK architecture decisions for the explorer

## Decisions Made

### 1. Stack decomposition: 6 layer-based stacks

**Decision:** NetworkStack → StorageStack → ComputeStack → DeliveryStack → MonitoringStack → CiStack (layering / dependency flow)

**Why:** Arrow order expresses architectural layering and dependencies (network → data → compute → delivery → monitoring), while CiStack is infrastructural and must be deployed first to create the OIDC provider/roles. This separation still cleanly splits stateful (Storage) from stateless (Compute), and avoids both a single-stack bottleneck and over-granular per-service stacks. Team of 2 doesn't need more than 6 stacks.

**Stateful/stateless split:** StorageStack has termination protection. ComputeStack can be freely destroyed/recreated.

### 2. Nx integration: custom targets with `run-commands` executor

**Decision:** Don't use `nx-serverless-cdk` plugin. Use `nx:run-commands` executor wrapping `cdk synth`, `cdk deploy`, `cdk diff` with `dependsOn: ["^build"]`.

**Why:** Community plugin is an unnecessary abstraction layer for a team that knows CDK. `run-commands` is simpler, debuggable, and doesn't lock us into a plugin's conventions. `dependsOn: ["^build"]` ensures all app builds complete before CDK synth.

### 3. Rust Lambda bundling: pre-built binary with `Code.fromAsset()`

**Decision:** Build Rust binary in CI via `cargo lambda build --release --arm64 --output-format zip`, reference from CDK with `Code.fromAsset()`.

**Why:** Nx manages ALL builds (TypeScript + Rust). No build during `cdk synth`. Simpler to debug, cache, and reason about. `cargo-lambda-cdk` is available as fallback if pre-build becomes too complex.

### 4. Environment configuration: TypeScript config module

**Decision:** Typed `EnvironmentConfig` interface with `staging.ts` and `production.ts` files at `infra/aws-cdk/lib/config/`.

**Why:** Type-safe, IDE autocomplete, compile-time validation of 20+ config values. Better than CDK context (flat, untyped) or env vars (no structure). Account ID comes from `CDK_DEFAULT_ACCOUNT` — no hardcoding.

### 5. Schema migration: CDK Custom Resource Lambda

**Decision:** Drizzle Kit migration Lambda triggered as Custom Resource during CDK deploy. Runs AFTER RDS, BEFORE application Lambdas.

**Why:** Runs within CDK deploy flow with automatic ordering via `addDependency()`. No extra service (CodeBuild). Forward-only migrations. CloudFormation rolls back if migration fails.

### 6. GitHub Actions: OIDC + `cdk diff` on PR + `cdk deploy` on merge

**Decision:**

- OIDC role assumption per ADR-0001 (separate staging/prod roles)
- `cdk diff` posted as PR comment via `corymhall/cdk-diff-action`
- `cdk deploy` on merge to develop (staging auto-deploy)
- Production deploy requires GitHub Environment approval gate

### 7. OIDC provider: `aws-cdk-github-oidc` construct

**Decision:** Use `aws-cdk-github-oidc` library in CiStack to create OIDC provider and deploy roles.

**Why:** CDK-managed, per ADR-0001. Roles scoped to repo + branch conditions.

### 8. Staging password protection: CloudFront Function with Basic Auth

**Decision:** CloudFront Function (not Lambda@Edge) for staging Basic Auth. Basic Auth secret is managed in Secrets Manager and provided to the function without resolving the secret value at synth time (e.g., via a deploy-time custom resource or by storing/injecting only a pre-hashed credential).

**Why:** CloudFront Functions are ~6x cheaper than Lambda@Edge, sub-ms latency, sufficient for Basic Auth. Staging-only mechanism doesn't justify Lambda@Edge complexity. Resolving Secrets Manager references at synth time would embed secret material into the synthesized template or deployed function code and must be avoided; any custom pattern that does so should be treated as a deliberate tradeoff with clear leak-risk acceptance.

### 9. Provisioned concurrency: API Lambda only in production

**Decision:** Only `apiLambdaProvisionedConcurrency` > 0 in production (start with 5). Rust Ledger Processor does NOT need it (100ms cold start). Event Interpreter does NOT need it (scheduled, not latency-sensitive).

### 10. S3 lifecycle: 7 days staging, 30 days production

**Decision:** `ledgerDataRetentionDays` in `EnvironmentConfig`. Values per architecture docs. `api-docs` bucket has no lifecycle rules.

### 11. ECR repository in ComputeStack

**Decision:** ECR repository for Galexie Docker images lives in ComputeStack alongside ECS Fargate task definitions.

**Why:** ECR is a compute dependency — tightly coupled to ECS. Same deployment lifecycle.

### 12. SQS DLQ for Ledger Processor in ComputeStack

**Decision:** SQS Dead Letter Queue for failed S3-triggered Lambda invocations. After max retries, the original S3 event is sent to DLQ for manual replay.

**Why:** Per architecture docs (task 0070) — failed XDR files must remain replayable.

### 13. ACM certificates in DeliveryStack

**Decision:** ACM certificates for CloudFront (us-east-1 required) and API Gateway (stack region). Managed in DeliveryStack.

### 14. Cross-compilation: cargo-lambda via pip3 in CI

**Decision:** Install `cargo-lambda` via `pip3 install cargo-lambda` in GitHub Actions. Use `dtolnay/rust-toolchain` for Rust. Local dev uses Zig cross-linker.

### 15. cdk.context.json committed to repo

**Decision:** Always commit `cdk.context.json` for deterministic synthesis. Not gitignored.

**Why:** AWS CDK best practice. Without it: non-deterministic synthesis, CI failures, team drift.

## Open Questions

1. **Rust Lambda in `apps/indexer-rs/` or `apps/indexer/`?** — If ADR-0002 accepted, the existing `apps/indexer` (TypeScript skeleton) should be replaced or renamed. Recommend `apps/indexer-rs/` as a new Nx project.
2. **`nx-serverless-cdk` reconsidered?** — if the team grows, the plugin's generators could accelerate scaffolding. For now, manual is simpler.

## Recommended CDK Dependencies

```json
{
  "dependencies": {
    "aws-cdk-lib": "^2.244.0",
    "constructs": "^10.0.0",
    "cargo-lambda-cdk": "^0.0.36",
    "aws-cdk-github-oidc": "^2.4.1"
  },
  "devDependencies": {
    "aws-cdk": "^2.244.0"
  }
}
```
