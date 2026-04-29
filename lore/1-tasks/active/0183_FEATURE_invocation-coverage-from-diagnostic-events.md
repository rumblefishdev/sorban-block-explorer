---
id: '0183'
title: 'Full Soroban invocation coverage via fn_call/fn_return diagnostic events'
type: FEATURE
status: active
related_adr: ['0029', '0033', '0034', '0026', '0030']
related_tasks: ['0167', '0182', '0173']
tags:
  [indexer, soroban, invocations, schema-migration, xdr-parser, observability]
links:
  - 'docs/architecture/database-schema/endpoint-queries/13_get_contracts_invocations.sql'
  - 'docs/architecture/database-schema/endpoint-queries/03_get_transactions_by_hash.sql'
  - 'docs/architecture/frontend/frontend-overview.md §6.4'
  - 'crates/xdr-parser/src/invocation.rs'
  - 'crates/indexer/src/handler/persist/staging.rs (~700)'
history:
  - date: 2026-04-29
    status: backlog
    who: fmazur
    note: >
      Task created. During E03 manual verification we found 53 % (7078/13308)
      of Soroban tx have 0 rows in `soroban_invocations_appearances` because
      the indexer extracts invocations exclusively from
      `InvokeHostFunctionOp.auth[].root_invocation`. Functions that don't
      call `require_auth()` (router patterns, contract-internal sub-calls,
      read-only views) leave the auth tree empty, so the appearance index
      misses them. stellar.expert reconstructs the same trees from
      `result_meta_xdr` `fn_call`/`fn_return` diagnostic events; this task
      lifts that approach into the indexer.
  - date: 2026-04-29
    status: active
    who: fmazur
    note: 'Promoted to active via /promote-task'
---

# Full Soroban invocation coverage via fn_call/fn_return diagnostic events

## Summary

Extend the indexer to populate `soroban_invocations_appearances` from the
**execution** invocation tree (reconstructed from `fn_call` / `fn_return`
diagnostic events in `result_meta_xdr`) **in addition to** the existing
auth-entry tree. Closes the 53 % coverage gap that leaves the
frontend-§6.4 "Normal mode contract-to-contract hierarchy" empty for
auth-less DeFi router txs (e.g. multi-hop swaps).

## Context

`crates/xdr-parser/src/invocation.rs:9-19` documents the current scope:

> Auth entries represent the authorization call graph and are the only
> reliably available **structured** tree in Soroban transactions.
>
> **Limitation:** Invocations that do not require caller authorization
> (e.g. read-only sub-calls, internal helper contracts) will not appear
> in the auth tree.

E03 manual verification (2026-04-29) measured the impact: **7078 / 13308
Soroban tx (53 %)** in the local 100-ledger test set have zero rows in
`soroban_invocations_appearances`. Concrete example: tx
`b7b51065e0a6830e684269c3d4e0c1c3dc76b0c66e97fc7d46fbd15c3b163235` is a
multi-hop swap (Phoenix → Aquarius pools) with ~12 nested calls visible
on stellar.expert; our DB has 0 invocation rows. `auth.len()` for that
tx is 0 because the user signs only the outer router call; all nested
calls execute under contract authority.

stellar.expert renders the full tree by reconstructing it from
`result_meta_xdr` → `diagnosticEvents` → `fn_call` / `fn_return`. Those
events are emitted by the host VM around every contract entry/exit and
form a depth-first stream that re-parses to a tree. They are part of
`v4.diagnostic_events`, currently dropped wholesale by `staging.rs:700`
as part of task **0182**'s fix for Contract-typed event leak.

The "future enhancement" mentioned in the indexer docstring is exactly
this task.

## Scope (confirmed with owner 2026-04-29)

- **Q2 / schema (β)** — add `caller_contract_id BIGINT` column to
  `soroban_invocations_appearances`, FK to `soroban_contracts(id)`.
  Mutually exclusive with `caller_id` via CHECK. Required because
  contract-to-contract callers can't be represented in the existing
  account-FK column.
- **Q3 / detail level (Minimal)** — keep `function_name`, `args`,
  `return_value`, `invocation_index`, `parent_invocation_id` out of DB
  (already overlaid from archive XDR per ADR 0029). Goal is **coverage**,
  not richer per-row data. One folded row per `(contract, tx)` like
  today, just with full population.
- **Q4 / backfill (deferred — pre-production)** — owner is pre-backfill,
  testing parser locally to flush bugs. No targeted backfill planned in
  this task. Reindex from clean DB will pick up the fix.

## Implementation Plan

### Step 1: Add diagnostic-event-driven invocation extractor

New function in `crates/xdr-parser/src/invocation.rs`:

```rust
pub fn extract_invocations_from_diagnostics(
    tx_meta: &TransactionMeta,
    transaction_hash: &str,
    ledger_sequence: u32,
    created_at: i64,
    tx_source_account: &str,
    successful: bool,
) -> Vec<ExtractedInvocation>
```

- Walks `v4.diagnostic_events` in order.
- Maintains a stack of currently-active invocations.
- On `fn_call(contract_id, function_name, args)` event: push frame, emit
  ExtractedInvocation row with caller = top-of-stack contract or
  tx_source_account if stack is empty.
- On `fn_return(function_name, return_value)` event: pop frame.
- On execution trap mid-call (no matching `fn_return`): close out
  remaining stack at end (mark return_value Null).
- Edge case: events with non-Contract source (e.g. host system events)
  are skipped.

Topic format reference (from soroban-host source):

- `fn_call` topic: `["fn_call", contract_id_to_be_called, function_name, args]`
- `fn_return` topic: `["fn_return", function_name, return_value]`

### Step 2: Wire diagnostic-tree into existing extract_invocations

Either:
(a) extend `extract_invocations()` to merge auth-tree + diag-tree
internally, dedupe by (contract, caller, depth-position), OR
(b) leave `extract_invocations()` as-is (auth only) and call both from
the indexer staging layer, merging there.

**Decision deferred to implementation time** — which one keeps the
diff smaller against task 0182's recently-merged staging layout wins.

### Step 3: Schema migration

New migration file in `crates/db-partition-mgmt/migrations/` (or wherever
sqlx migrations live in this repo — verify):

```sql
ALTER TABLE soroban_invocations_appearances
  ADD COLUMN caller_contract_id BIGINT
    REFERENCES soroban_contracts(id);

ALTER TABLE soroban_invocations_appearances
  ADD CONSTRAINT ck_sia_caller_xor
    CHECK (
      (caller_id IS NULL AND caller_contract_id IS NULL)
      OR (caller_id IS NOT NULL AND caller_contract_id IS NULL)
      OR (caller_id IS NULL AND caller_contract_id IS NOT NULL)
    );
```

NULL/NULL allowed for unknown caller (shouldn't happen in normal
operation but defensive).

PK stays `(contract_id, transaction_id, ledger_sequence, created_at)` —
caller doesn't enter natural identity (a row is one fold per call site).
Note: this means if the same `(contract, tx)` pair has TWO callers
(e.g. router CC2J calls pool CBHC, then later pool CBHC calls itself or
something), we need to decide fold semantics. Likely: **first caller
wins** (matches existing `amount` fold semantic), or extend PK to
include caller. Decide during implementation.

### Step 4: Persist layer

`crates/indexer/src/handler/persist/staging.rs` and
`crates/indexer/src/handler/persist/write.rs`:

- Update `InvRow` struct to carry both caller variants (account vs
  contract).
- Update `insert_invocations` SQL to populate the new column.
- Verify `idx_sia_*` indexes still cover the read paths in E13 / E03 /
  E02 (Statement B). If not, add.

### Step 5: Tests

- Unit tests in `crates/xdr-parser/src/invocation.rs` for the new
  diagnostic-event walker (use the same fixture style as the existing
  auth-tree tests).
- Integration test in `crates/indexer/tests/persist_integration.rs`:
  feed a known tx hash (suggest
  `b7b51065e0a6830e684269c3d4e0c1c3dc76b0c66e97fc7d46fbd15c3b163235`
  from local archive — already at
  `.temp/FC4DB5FF--62016000-62079999/FC4DB59C--62016086.xdr.zst`) and
  assert the resulting `soroban_invocations_appearances` row count
  matches the expected ~12 contract touches.
- Regression check on task 0182: verify Contract-typed leak from
  diagnostic_events is still dropped from
  `soroban_events_appearances` (we're using diag for invocations, not
  for events — events stay consensus-only).

### Step 6: SQL reference + docs update

- `docs/architecture/database-schema/endpoint-queries/03_get_transactions_by_hash.sql`
  statement F header: drop the "see ADR 0034 — auth tree only" framing;
  add note that the table now reflects execution tree.
- `docs/architecture/database-schema/endpoint-queries/13_get_contracts_invocations.sql`
  same.
- ADR 0034 update or supersede — current ADR states the table is auth-tree
  derived. Decide if we update in place or write a new ADR superseding.
- `docs/architecture/backend/backend-overview.md` if it mentions the
  auth-only limitation.

## Acceptance Criteria

- [ ] `extract_invocations_from_diagnostics` implemented + unit tests
- [ ] `soroban_invocations_appearances` migration adds
      `caller_contract_id` + CHECK constraint
- [ ] Persist layer writes the new column for both auth-tree and
      diag-tree origins
- [ ] On a fresh local index of the 100-ledger test set, the count of
      Soroban tx with zero invocation rows drops from ~7078 to near zero
      (target: only tx with no diagnostic events at all)
- [ ] Test tx `b7b510…3235` produces ~12 invocation rows touching all
      4 contracts seen in events
- [ ] Task 0182's Contract-event-leak fix is preserved
      (diagnostic events still dropped from `soroban_events_appearances`)
- [ ] **Docs updated** — files E03 / E13 SQL headers updated; ADR 0034
      either superseded or amended; `backend-overview.md` if it
      enumerates the limitation

## Notes

- **Diagnostic events availability**: confirmed in
  `staging.rs` comment (~line 683): "When diagnostic mode is enabled
  (default for Galexie's captive-core), `v4.diagnostic_events` holds…".
  Galexie is the project's ledger-blob source, so diagnostic events are
  reliably present.
- **Pre-production status**: owner is pre-backfill, testing parser
  locally. No production data to migrate. Reindex on clean DB picks up
  the fix.
- **Out of scope** (no auto-spawn per owner standing rule): targeted
  backfill of partial production data, richer per-row data
  (`function_name`, `depth`, `invocation_index`) for serving Normal-mode
  tree directly from DB without archive overlay (would conflict with
  ADR 0029 boundary).
