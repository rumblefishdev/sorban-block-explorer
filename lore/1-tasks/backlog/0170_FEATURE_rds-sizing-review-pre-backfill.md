---
id: '0170'
title: 'Infra: RDS instance type sizing review before mainnet backfill'
type: FEATURE
status: backlog
related_adr: ['0011', '0019', '0037']
related_tasks: ['0045', '0130', '0145']
tags: [layer-infra, rds, performance, capacity-planning, phase-pre-backfill]
links: []
history:
  - date: 2026-04-28
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from PR #125 (task 0045) review. Current
      `dbInstanceClass: db.t4g.micro` insufficient for 10.5M accounts +
      11.6M ledgers backfill workload at Stellar mainnet scale; right-sizing
      must precede task 0130 (historical partitions) and task 0145
      (backfill-runner).
---

# Infra: RDS instance type sizing review before mainnet backfill

## Summary

Both `infra/envs/staging.json` and `infra/envs/production.json` set
`dbInstanceClass: t4g.micro` (1 GB RAM, 2 vCPU burstable, 20 GB initial gp3
storage). Sustained mainnet workload — 10.5M accounts (per stellar.expert),
91k contracts (per stellarchain.io), and ~78M transactions per monthly
partition at current Stellar activity — exceeds what t4g.micro can serve
without CPU credit exhaustion and disk I/O throttling, especially during
concurrent indexer writes.

Measure expected RDS load against the staging compose stack populated to
representative scale, project to 11.6M ledgers, recommend instance type and
gp3 IOPS/throughput overrides, and update both env config files before
task 0145 backfill-runner kicks off mainnet ingest.

## Context

Surfaced during PR #125 (`/network/stats`) review. Even after aligning the
endpoint with the canonical SQL deliverable from task 0167
(`docs/architecture/database-schema/endpoint-queries/01_get_network_stats.sql`)
— `pg_class.reltuples` for counts, `ledgers.transaction_count` aggregate
for TPS — the underlying RDS instance is still the bottleneck for any
endpoint touching partition-scale data: indexer writer contention plus
range scans on 78M-row partitions outpaces what 1 GB shared buffers and
2 vCPU burst credits can sustain.

This is a hard prerequisite for backfill — running task 0130
(historical partition creation) and task 0145 (backfill ingest) against
the current `t4g.micro` will exhaust CPU credits within minutes and stall
the entire run.

## Implementation Plan

1. **Cost model.** Extract per-query expected cost from
   [ADR 0019 sizing assumptions](../../2-adrs/0019_schema-snapshot-and-sizing-11m-ledgers.md)
   and the schema in [ADR 0037](../../2-adrs/0037_current-schema-snapshot.md).
2. **Load harness.** Populate compose-stack DB to representative scale
   (~1M ledgers via selective testnet backfill).
3. **Endpoint suite measurement.** Run `/network/stats`, `/transactions`,
   `/transactions/:hash`, `/accounts/:id`, `/accounts/:id/transactions`,
   `/contracts/:id/events`, `/contracts/:id/invocations`, and
   `/liquidity-pools` under load. Capture `pg_stat_statements` and RDS
   CloudWatch metrics (CPU credits, BurstBalance, IOPS, throughput,
   read/write latency, FreeableMemory).
4. **Recommendation.** Baseline instance type (likely `r6g.large` or
   `r6g.xlarge`), gp3 IOPS, throughput; read replica strategy decision
   (Aurora reader endpoint vs plain RDS replica, or none for now).
5. **Apply.** Update `infra/envs/{staging,production}.json` and CDK
   stacks (`api-gateway-stack.ts`, `rds-stack.ts`) as needed.

## Acceptance Criteria

- [ ] Cost-vs-capacity matrix produced for at least 3 instance options
- [ ] `dbInstanceClass`, gp3 `iops`, gp3 `throughput` updated in both
      `staging.json` and `production.json`
- [ ] Read replica strategy decided and documented (yes/no, type, when)
- [ ] Sign-off explicitly coordinated with task 0130 partition creation
      timing — no ledger inserted into RDS before this task is done
- [ ] **Docs updated** — `docs/architecture/infrastructure/*.md` reflects
      the new sizing per [ADR 0032](../../2-adrs/0032_docs-architecture-evergreen-maintenance.md)

## Notes

Block task 0145 (backfill-runner) kickoff on this task's completion. If
the team decides to run a smaller scale validation first (e.g. last 30
days only), document the temporary right-sizing as a separate sub-step
and revert before full historical backfill.

The configured `db.t4g.micro` is fine for development compose stacks and
empty CI runs — this task does not touch local dev workflow, only the
deployed staging/production envs.
