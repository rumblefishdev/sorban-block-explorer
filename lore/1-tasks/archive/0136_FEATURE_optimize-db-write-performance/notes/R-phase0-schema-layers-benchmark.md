---
status: mature
spawned_from: '0136'
---

# Phase 0: Schema Layers Benchmark

Benchmark: 100 ledgers (62016000–62016099), fresh insert, 4 rounds.
Each round drops one layer of constraints/indexes cumulatively.

## Results

| Round                                    | Avg/ledger | Delta | Savings |
| ---------------------------------------- | ---------- | ----- | ------- |
| BASELINE (all constraints + indexes)     | 360ms      | —     | —       |
| NO FK                                    | 275ms      | -85ms | 24%     |
| NO FK + NO UNIQUE                        | 238ms      | -37ms | 10%     |
| NO FK + NO UNIQUE + NO INDEXES (PK only) | 202ms      | -36ms | 10%     |

## Per-layer cost breakdown

| Layer             | Cost        | Worth dropping?                                                                    |
| ----------------- | ----------- | ---------------------------------------------------------------------------------- |
| FK                | 85ms (24%)  | Yes — largest gain, zero value with controlled pipeline                            |
| UNIQUE            | 37ms (10%)  | Yes — idempotency at application level (skip processed ledger)                     |
| GIN + B-tree      | 36ms (10%)  | Yes, but requires rebuild after backfill (hours at 11M)                            |
| Heap + PK + JSONB | 202ms (56%) | This is the "ceiling" — cannot be bypassed without COPY protocol or data reduction |

## Decision (2026-04-13)

After senior review: **stay on BASELINE**. No schema changes for now.

Data is preserved here for future reference if the team revisits
optimization. The 202ms floor means <100ms target is not achievable
through schema changes alone — would require COPY protocol, parallel
workers, or data volume reduction.
