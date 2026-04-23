---
id: '0158'
title: 'REFACTOR: soroban_invocations → soroban_invocations_appearances (ADR 0033 analogue)'
type: REFACTOR
status: active
related_adr: ['0033', '0029', '0027', '0030']
related_tasks: ['0157']
tags: [layer-backend, layer-db, effort-medium, schema, s3-read-path]
links:
  - lore/2-adrs/0033_soroban-events-appearances-read-time-detail.md
  - lore/2-adrs/0029_abandon-parsed-artifacts-read-time-xdr-fetch.md
  - lore/2-adrs/0027_post-surrogate-schema-and-endpoint-realizability.md
  - lore/1-tasks/archive/0157_REFACTOR_soroban-events-appearances-adr-0033.md
history:
  - date: '2026-04-23'
    status: backlog
    who: fmazur
    note: >
      Spawned from task 0157 / ADR 0033 review. `soroban_invocations` is
      the direct analogue of `soroban_events` — per-Soroban-tx detail
      table whose content is fully recoverable from the transaction
      envelope + meta. Applying the same appearance-index pattern is the
      remaining piece that brings every event/invocation-bearing endpoint
      onto a single read model.
  - date: '2026-04-23'
    status: active
    who: fmazur
    note: >
      Activated as current task immediately after 0157 closed. Work picks
      up on its own branch and PR; the ADR 0033 schema + write-path PR
      for events lands separately first as the pattern precedent.
---

# REFACTOR: soroban_invocations → soroban_invocations_appearances

## Summary

Collapse the 9-column `soroban_invocations` table into an appearance index
`soroban_invocations_appearances` mirroring the `soroban_events_appearances`
shape delivered by task 0157 / ADR 0033. Per-row detail (function name,
caller, success flag, per-node invocation index) moves to read-time XDR
fetch through `xdr_parser::extract_invocations`. The DB becomes a pointer
to `(contract, tx, ledger, amount)` trios.

## Status: Backlog

**Current state:** Not started. Proposed after task 0157 closed the same
refactor for `soroban_events`. Follow-up ADR required (tentative number
TBD — reuses ADR 0033's pattern but different table and different
endpoints, so deserves its own decision record).

## Context

`soroban_invocations` today stores one row per node in the Soroban
invocation tree — per ADR 0027 §10:

```
id, transaction_id, contract_id, caller_id, function_name, successful,
invocation_index, ledger_sequence, created_at
```

with indexes on `(contract_id, created_at DESC)` and `(caller_id, created_at DESC)`.

Every per-row column (except the identity/partitioning columns
`transaction_id`, `contract_id`, `ledger_sequence`, `created_at`) is already
produced by `xdr_parser::extract_invocations` from the transaction envelope
(`SorobanAuthorizationEntry.root_invocation` + `InvokeHostFunctionOp.args`)
plus `SorobanTransactionMeta.return_value`. The parser output is richer
than what the DB currently keeps — it carries the full tree structure,
`function_args`, `return_value`, and depth — none of which are in the DB
today but all of which the read-path needs.

The table scales 1:1 with Soroban-invoking transactions (empirically ~1–3
invocation-tree nodes per tx that calls a contract). At the sample ADR
0033 §Context uses, `soroban_invocations` is the second-largest Soroban-
domain consumer after `soroban_events`; proportionally it will continue
to be so post-refactor.

## Scope

### In scope

1. **Follow-up ADR draft** — decision record for this table, explicitly
   referencing ADR 0033's pattern and calling out the table-specific
   trade-offs (particularly the `caller_id` index question; see
   Open Questions).
2. **Schema rewrite in place** —
   `crates/db/migrations/0004_soroban_activity.sql` §10 replaced with
   `soroban_invocations_appearances (contract_id, transaction_id,
ledger_sequence, amount, created_at)`; PK, FK to `transactions
(id, created_at) ON DELETE CASCADE`, FK to `soroban_contracts(id)`,
   monthly partitioning on `created_at` preserved. Drop
   `uq_soroban_invocations_tx_index` from
   `20260421000100_replay_safe_uniques.up/down.sql` (new PK covers
   replay idempotency).
3. **Indexer write path** — rewrite `insert_invocations` in
   `crates/indexer/src/handler/persist/write.rs` as aggregate-by-trio,
   mirror `insert_events` structure. `amount = count of tree nodes
for (contract, tx, ledger)`. Diagnostic-analogue filter: none —
   all invocation nodes count (there is no invocation equivalent of
   diagnostic events).
4. **Staging cleanup** — simplify `InvRow` to the identity fields;
   drop `caller_str_key`, `function_name`, `successful`,
   `invocation_index` from the row. Update `Staged::prepare` to stop
   resolving caller StrKeys for this table (but **keep the
   participant registration** if caller still counts as a tx
   participant — see Open Questions).
5. **Domain cleanup** — remove dead `domain::soroban::SorobanInvocation`
   struct and related doc comments (same treatment 0157 gave to
   `SorobanEvent`). Check grep for real callers first.
6. **Partition management + benches** — update `db-partition-mgmt`
   `TIME_PARTITIONED_TABLES` (rename `soroban_invocations` →
   `soroban_invocations_appearances`); update `backfill-bench` default
   partition list.
7. **ADR 0021 coverage matrix update** — rows E11 / E12 / E13 (contract
   page → invocations / inner-calls) move to "DB appearances + S3
   detail" category, same shape as E14.
8. **ADR 0027 §10 marker** — add "superseded for this table by
   [follow-up ADR]" marker pointing to the new ADR. ADR 0027
   otherwise stands.
9. **Tests** — extend `persist_integration` to assert
   `SUM(invocations.amount) = ingested tree-node count`, mirroring the
   events assertion added in 0157.
10. **Size-measurement note** — baseline the new table size against
    the same indexer-run sample used for 0157's measurement note
    (when that lands).

### Out of scope

- **Read-path wire-up (handlers E11 / E12 / E13).** Same reason as
  0157: no `AppState` / `IntoResponse` / routing module in API yet.
  Schema substrate lands here; handlers land with the API bootstrap.
- **Caching.** Same "measure first" rule as ADR 0029.
- **Any change to non-invocation Soroban tables.**
  `soroban_contracts`, `wasm_interface_metadata` untouched.

## Implementation Plan

Mirrors 0157's 10-step sequence exactly — the analogue is deliberate.
Each step below corresponds 1:1 to the matching step in 0157 so the
review is shaped the same way.

1. Draft the follow-up ADR.
2. Rewrite migrations in place.
3. Rewrite `insert_invocations` as aggregate + `ON CONFLICT DO NOTHING`.
4. Simplify `InvRow`, cleanup staging.
5. Delete dead `domain::SorobanInvocation` + updates doc comments.
6. Update partition-mgmt + backfill-bench table lists.
7. ADR 0021 matrix rows E11/E12/E13 rewritten.
8. ADR 0027 §10 superseded marker.
9. Integration test for aggregate `amount`.
10. Size-measurement baseline.

## Acceptance Criteria

- [ ] `soroban_invocations_appearances` matches the follow-up ADR's
      §Decision.1 DDL; `soroban_invocations` no longer exists in
      migrations or code.
- [ ] `insert_invocations` aggregates per
      `(contract, tx, ledger, created_at)`; `persist_integration`
      asserts `SUM(amount)` equals the ingested tree-node count.
- [ ] No compile-time reference to `caller_id`, `function_name`,
      `successful`, or `invocation_index` in the DB / indexer / API
      code for this table.
- [ ] ADR 0021 coverage matrix updated (E11 / E12 / E13 rows);
      ADR 0027 §10 carries a superseded-for-this-table marker;
      follow-up ADR `accepted`.
- [ ] `cargo build --workspace`, `cargo test --workspace --lib`,
      `cargo clippy --workspace --all-targets -- -D warnings`, and
      `cargo fmt --all -- --check` all green.
- [ ] Read-path wire-up (E11 / E12 / E13 handlers) — **deferred**
      (API bootstrap out of scope; handled with the API follow-up
      that also lands E3 / E10 / E14 from 0157).

## Open Questions

1. **`caller_id` retention vs. drop.** Current schema indexes
   `(caller_id, created_at DESC)` — supports "list contract
   invocations caused by account X". If this query pattern is
   actually wired on any account-page endpoint, the appearance index
   may need a `caller_id BIGINT` column kept alongside the trio (so
   the shape becomes 5 user-facing columns, not 4). Decide by
   grepping API design docs + ADR 0021 account-page rows before the
   ADR is drafted. Default if undecided: keep it; the column is
   cheap.
2. **Tree nodes vs. tx-level count for `amount`.** Each invocation
   is a tree; the indexer emits one `ExtractedInvocation` per node
   (see `xdr_parser::types::ExtractedInvocation.depth`). The natural
   analogue to events is "one appearance row per (contract, tx,
   ledger); `amount` = tree nodes for that trio". This matches the
   current DB row count (one row per tree node today). Alternative:
   `amount = 1` per tx-level root call and drop the depth dimension.
   Former keeps parity with events and is the default choice unless
   the UX for E12 ("expand inner calls") treats nodes differently.
3. **Diagnostic-analogue filter.** Events have a diagnostic-kind
   filter at ingest (drop diagnostic from DB, keep only contract +
   system). Invocations have no such split — the parser emits every
   tree node. No filter needed at ingest; all nodes count toward
   `amount`.
4. **Inner-tx fee-bump contracts.** `transactions` distinguishes
   outer vs. inner via `inner_tx_hash`. Appearance rows inherit the
   outer's `transaction_id`. Make sure the count doesn't double-count
   inner calls that already appear under the outer tx's id. (Same
   invariant as events; should fall out naturally.)

## Notes

- Pure analogue of 0157. No new parser code needed —
  `xdr_parser::extract_invocations` already exists and is the sole
  source of per-invocation detail.
- Risk is lower than 0157 because the read-path consumers (E11 /
  E12 / E13) are not yet wired either — schema change is clean
  substrate with no API regression surface.
- The follow-up ADR should explicitly link ADR 0033 as the pattern
  precedent and focus its own text on table-specific trade-offs
  rather than re-justifying the overall approach.
