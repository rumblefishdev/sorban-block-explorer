---
id: '0169'
title: 'inner_tx_hash population + operations_appearances.amount semantics audit'
type: BUG
status: active
related_adr: ['0037']
related_tasks: ['0167', '0168', '0163']
tags: [indexer, schema, api, fee-bump, soroban, audit]
links:
  - 'docs/architecture/database-schema/endpoint-queries/02_get_transactions_list.sql'
  - 'lore/1-tasks/archive/0167_FEATURE_endpoint-sql-query-reference-set.md'
  - 'lore/1-tasks/archive/0168_BUG_envelope-tx-processing-misalignment.md'
history:
  - date: 2026-04-28
    status: backlog
    who: fmazur
    note: >
      Spawned from manual E02 verification. Two findings against Horizon
      mainnet on ledger 62016099 that need investigation + potential fix.
  - date: 2026-04-28
    status: active
    who: fmazur
    note: 'Promoted to active via /promote-task'
---

# inner_tx_hash population + operations_appearances.amount semantics audit

## Summary

Two findings surfaced during manual verification of the
`GET /transactions` endpoint against Horizon mainnet. Each may be a real
indexer/schema bug or a deliberate-but-undocumented choice; this task
investigates both, decides, and either fixes or documents.

## Context

E02 was verified on 50 rows from ledger `62016099`. 4 rows hand-checked
against Horizon: 3 fully aligned, 1 (the fee-bump audit row
`358ef42d…`) revealed two remaining discrepancies after lore-0168.

### Finding 1 — `transactions.inner_tx_hash` NULL on fee-bump rows

For the audit row `358ef42d9840d91554a46d69be7c7fee8f8f4305379ab6ed614e4ea9ae4e75dc`:

- **Horizon** reports `is_fee_bump_transaction = true`, `inner_transaction_hash = 12021959a49f62ec43b6985a22682ca63104c4a99641a1f83e0986baf15b266d`.
- **DB** has `inner_tx_hash = NULL`.

lore-0168 fixed `source_id` for fee-bumps (the inner-tx source is now
correct) but did not touch `inner_tx_hash` population. The column shape
in ADR 0037 implies it should be set when the row IS a fee-bump
envelope. Need to confirm whether the indexer's
`extract_transactions` path computes the inner hash for fee-bump
variants, or silently leaves it `NULL`.

### Finding 2 — `operations_appearances.amount` dual semantics

E02's projection `primary_op_amount = pop.amount` is `1` on every
soroban INVOKE_HOST_FUNCTION row in the sample. That's not stroops
— per ADR 0037 §7 / task 0163 the column is the **fold count of
duplicate appearances**, not a value amount. The column is reused
across op types with different meaning:

- classic transfer ops: stroop amount (real value).
- soroban / appearance-only rows: count of folded duplicates (always 1
  for the canonical case).

The frontend reading `primary_op_amount` from E02's response cannot
distinguish without checking `op_type`. 0167's "Issues Encountered"
already flags a possible rename `amount` → `appearance_count`; this
task either ships that rename (schema migration + ADR) or documents
the dual semantics in ADR 0037 / endpoint-queries README so consumers
don't misinterpret.

## Implementation Plan

### Step 1: investigate inner_tx_hash

- read `crates/xdr-parser/src/envelope.rs` and `transaction.rs` —
  does `extract_transactions` set `inner_tx_hash` for `TxFeeBump`
  envelopes?
- check the ingestion path in `crates/indexer/src/process.rs` and
  `staging.rs` — is the column written?
- audit existing data: `SELECT COUNT(*) FROM transactions WHERE
inner_tx_hash IS NULL AND <fee-bump heuristic>;` to gauge scope.
- decide: bug (fix + reindex) or expected (drop column / mark
  optional in ADR 0037).

### Step 2: decide on operations_appearances.amount

- Option A — rename to `appearance_count`. Schema migration, indexer
  update, all `oa.amount` SQL refs renamed (E02, E03, E07, E10, E13,
  E20, E22 + Rust handlers). New ADR documenting the rename.
- Option B — keep dual semantics, document. Add a note in ADR 0037
  §7 + endpoint-queries README. Update E02 header to say
  `primary_op_amount` is meaningful only for classic transfer op
  types; NULL it out in projection for soroban/appearance-only types.
- Owner picks.

### Step 3: ship the chosen fixes

- For Finding 1: indexer fix + targeted backfill of
  `inner_tx_hash` over existing partitions, OR a column-drop ADR if
  it's deliberate.
- For Finding 2: schema migration + reindex (Option A) OR doc-only
  PR (Option B).

## Acceptance Criteria

- [ ] **Finding 1 resolved.** Either: indexer populates
      `transactions.inner_tx_hash` for fee-bump rows AND existing
      rows are backfilled; OR an ADR documents the column as
      intentionally-NULL with rationale.
- [ ] **Finding 2 resolved.** Either: rename + migration shipped
      with a new ADR; OR doc-only update to ADR 0037 §7 + the
      endpoint-queries README + E02 projection clarification.
- [ ] Re-running the E02 verification on the audit row no longer
      surfaces either discrepancy (or surfaces them with a
      doc-pointer).
- [ ] **Docs updated** — ADR 0037 (schema) and/or
      endpoint-queries README updated to reflect chosen direction
      per [ADR 0032](../../2-adrs/0032_docs-architecture-evergreen-maintenance.md).

## Notes

- Audit row for regression check:
  `358ef42d9840d91554a46d69be7c7fee8f8f4305379ab6ed614e4ea9ae4e75dc`
  on ledger `62016099` — fee-bump tx with Horizon-reported
  `inner_transaction_hash = 12021959a49f62ec43b6985a22682ca63104c4a99641a1f83e0986baf15b266d`.
- The two findings are independent; this task bundles them because
  both surfaced from the same E02 verification pass and both fall
  into the "is this a bug or a doc gap?" bucket. They may split
  into two PRs at implementation time.
