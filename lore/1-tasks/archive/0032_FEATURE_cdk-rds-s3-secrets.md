---
id: '0032'
title: 'CDK: RDS PostgreSQL, RDS Proxy, S3 bucket, Secrets Manager'
type: FEATURE
status: completed
related_adr: ['0006']
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
  - date: 2026-03-31
    status: active
    who: fmazur
    note: 'Activated task for implementation'
  - date: 2026-03-31
    status: completed
    who: fmazur
    note: >
      Implemented 3 micro-stacks: RdsStack, LedgerBucketStack (api-docs
      bucket removed — Swagger served from API). 3 new stack files,
      shared ports.ts, ADR 0006 (no S3 lifecycle). Updated NetworkStack
      to move RDS SG to RdsStack (cross-stack cycle fix). Updated 5
      backlog tasks and 2 architecture docs to reflect api-docs removal.
---

# CDK: RDS PostgreSQL, RDS Proxy, S3 bucket, Secrets Manager

## Summary

Storage infrastructure: RDS PostgreSQL with RDS Proxy for connection pooling, S3 bucket for ledger XDR files, and Secrets Manager for database credentials. Swagger UI served directly from the API (NestJS `/docs` endpoint).

## Status: Completed

## Acceptance Criteria

- [x] RDS PostgreSQL instance is defined in the private subnet with appropriate engine and storage
- [x] RDS Proxy is defined and configured for connection pooling with Secrets Manager integration
- [x] All Lambda database access is routed through RDS Proxy (no direct RDS connections)
- [x] stellar-ledger-data S3 bucket is defined with blocked public access
- [ ] S3 event notification triggers Ledger Processor Lambda (deferred to task 0033 — Lambda does not exist yet)
- [x] Secrets Manager stores RDS credentials with automatic 30-day rotation for production
- [x] Production: KMS encryption on RDS and S3 ledger-data bucket, TLS enforced on RDS
- [x] Staging: SSE-S3 acceptable, TLS optional
- [x] Production: automated backups, PITR, deletion protection enabled
- [x] No read replica at launch (documented as future addition based on CPU threshold)
- [x] RDS definition structured so Single-AZ to Multi-AZ promotion requires only a configuration change
- [x] Staging and production use separate RDS instances (no shared database)

## Implementation Notes

### Files created

| File                                                 | Purpose                                                    |
| ---------------------------------------------------- | ---------------------------------------------------------- |
| `src/lib/stacks/rds-stack.ts`                        | RDS PostgreSQL, RDS Proxy, Secrets Manager, KMS            |
| `src/lib/stacks/ledger-bucket-stack.ts`              | S3 bucket for XDR files                                    |
| `src/lib/ports.ts`                                   | Shared port constants (POSTGRESQL, HTTPS, STELLAR_OVERLAY) |
| `lore/2-adrs/0006_no-s3-lifecycle-on-ledger-data.md` | ADR: no lifecycle rules on ledger bucket                   |

### Files modified

| File                                            | Change                                                                       |
| ----------------------------------------------- | ---------------------------------------------------------------------------- |
| `src/lib/stacks/network-stack.ts`               | Moved RDS SG to RdsStack (cross-stack cycle fix), egress to RDS via VPC CIDR |
| `src/lib/types.ts`                              | Added storage config fields (dbInstanceClass, dbAllocatedStorage, etc.)      |
| `src/lib/app.ts`                                | Wired RdsStack and LedgerBucketStack                                         |
| `src/index.ts`                                  | Re-exports new stacks                                                        |
| `envs/staging.json`                             | Added staging storage values                                                 |
| `envs/production.json`                          | Added production storage values                                              |
| `Makefile`                                      | Added deploy targets per stack                                               |
| `lore/1-tasks/backlog/0035,0038,0040,0042,0057` | Removed api-docs S3 bucket references                                        |
| `docs/architecture/*.md`                        | Updated to reflect Swagger from API, not S3                                  |

### CDK synth output

**Staging:** 3 stacks (Network, Rds, LedgerBucket)
**Production:** 3 stacks + KMS key + secret rotation

## Design Decisions

### From Plan

1. **RDS Proxy on both environments**: Lambda burst during testnet backfill
   justifies Proxy even on staging (~$20/mo).

2. **Secrets Manager with DatabaseSecret**: CDK auto-generates password,
   never in code. `Credentials.fromSecret()` pattern from reference project.

3. **GP3 storage**: Better price/performance than GP2. Auto-scaling capable.

### Emerged

4. **api-docs S3 bucket removed**: Swagger UI served from NestJS `/docs`
   endpoint — simpler, less infra, always in sync with API code.
   Updated 5 backlog tasks and 2 architecture docs.

5. **RDS SG moved from NetworkStack to RdsStack**: Cross-stack cyclic
   dependency. CDK can't resolve SG references bidirectionally between
   stacks. Fix: Lambda/ECS egress to RDS uses VPC CIDR instead of
   SG-to-SG reference.

6. **No S3 lifecycle rules (ADR 0006)**: Early development — unknown
   replay/debugging needs. Storage costs low. Revisit when costs grow.

7. **Micro-stacks pattern**: Senior feedback — separate stack per resource
   (rds-stack, ledger-bucket-stack) instead of monolithic StorageStack.
   Follows reference project pattern. Enables independent deploy/rollback.

8. **`kmsEncryption` as production hardening flag**: Controls KMS, TLS
   enforcement, secret rotation, and removal policy. Single boolean
   instead of multiple flags.

9. **Shared `ports.ts`**: Port constants extracted to single file —
   avoids duplication across stacks. New stacks import from same source.

10. **`t4g.micro` instead of `t3.micro` for staging**: ARM instance,
    cheaper than x86 equivalent. 1 GB RAM sufficient for staging.

## Issues Encountered

- **Cross-stack cyclic dependency**: CDK throws `DependencyCycle` when
  StorageStack references NetworkStack SGs and vice versa. Fix: RDS SG
  lives in RdsStack, Lambda/ECS egress uses VPC CIDR for port 5432.

- **Conditional parameterGroup with spread syntax**: `...(config.kmsEncryption && { parameterGroup: ... })`
  — TypeScript accepts spreading `false` as no-op. Works but non-obvious pattern.

## Future Work

- S3 event notification for Ledger Processor Lambda (task 0033)
- S3 lifecycle rules when storage costs become a concern (ADR 0006)
- Read replica when CPU exceeds monitoring threshold (task 0036)
