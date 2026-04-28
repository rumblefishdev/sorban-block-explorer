# Audit harness — saved run records

Each markdown file here is the captured output of one audit harness run
against a specific dataset. The runs are append-only — newer runs go
into new files; old reports stay as historical evidence.

## File naming

`<date>-<phase>-<dataset>.md` — `2026-04-28-phase1-30k.md` is the
Phase 1 SQL invariant run from 2026-04-28 against the 30k ledger smoke.

## Why save these

1. **Bug spawn evidence** — the 0176/0177/0178/0179 task bodies
   reference specific violation counts and sample row keys; those
   numbers come from these reports. Lose the report and the task body
   becomes unverifiable.
2. **Regression locking** — once a fix lands, re-running the same
   phase against a re-indexed dataset is the way to declare the bug
   closed. Keep the pre-fix and post-fix reports side by side for a
   diff.
3. **Drift watching** — Phase 2a `balances` table emits 30-50 %
   mismatches that are *expected drift* (DB snapshot at
   last_updated_ledger vs Horizon current). Trend over time tells us
   whether drift is growing (real bug) or stable (acceptable).

## Naming conventions

- `phase1` — SQL invariants, no external dependency
- `phase2a` — DB ↔ Horizon API
- `phase2b` — DB ↔ Soroban RPC (deferred — RPC retention window
  too short for retroactive checks; pivot to stellar.expert API
  pending)
- `phase2c` — DB ↔ archive XDR re-parse

## How to capture a run

```bash
# Phase 1
DATABASE_URL=postgres://... \
  crates/audit-harness/run-invariants.sh \
  --out crates/audit-harness/reports/$(date +%Y-%m-%d)-phase1-<dataset>.md

# Phase 2a (per table — loop in shell)
for tbl in ledgers transactions accounts balances assets liquidity-pools; do
  cargo run -q -p audit-harness --bin horizon-diff -- \
    --table $tbl --sample 50 --concurrency 6
done > crates/audit-harness/reports/$(date +%Y-%m-%d)-phase2a-<dataset>.md

# Phase 2c
cargo run -q -p audit-harness --bin archive-diff -- \
  --table ledgers --sample 50 --concurrency 4 \
  > crates/audit-harness/reports/$(date +%Y-%m-%d)-phase2c-<dataset>.md
```
