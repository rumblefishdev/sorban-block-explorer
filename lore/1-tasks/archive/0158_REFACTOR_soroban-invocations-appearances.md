---
id: '0158'
title: 'REFACTOR: soroban_invocations → soroban_invocations_appearances (ADR 0033 analogue)'
type: REFACTOR
status: completed
related_adr: ['0034', '0033', '0029', '0027', '0030']
related_tasks: ['0157', '0159']
tags: [layer-backend, layer-db, effort-medium, schema, s3-read-path]
links:
  - lore/2-adrs/0034_soroban-invocations-appearances-read-time-detail.md
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
  - date: '2026-04-23'
    status: completed
    who: fmazur
    note: >
      All 10 planned steps delivered. ADR 0034 drafted + accepted. Schema
      rewritten in place (migration 0004 §10 + replay-safe uniques). Indexer
      write path collapsed to aggregate `insert_invocations` with
      `caller_id` retained as payload (see §Design Decisions → Emerged).
      InvRow slim, dead `domain::SorobanInvocation` struct deleted,
      partition-mgmt + backfill-bench renamed, ADR 0021 E11/E13 matrix
      rows rewritten, ADR 0027 §10 carries superseded-for-this-table
      marker to ADR 0034. Integration test now asserts
      `SUM(amount) = tree-node count`. Full Rust gate green (166 lib
      tests; workspace build, clippy `-D warnings`, fmt all clean).
      Read-path wire-up (E11 stats query, E13 handler) deferred with 0157
      as agreed. Bundled with task 0159 on separate branch.
---

# REFACTOR: soroban_invocations → soroban_invocations_appearances

## Summary

Collapsed the 9-column `soroban_invocations` table into an appearance
index `soroban_invocations_appearances` mirroring the
`soroban_events_appearances` shape from task 0157 / ADR 0033, with one
deliberate divergence: `caller_id` retained as an unindexed payload
column so that E11's `unique_callers` stat stays answerable via
`COUNT(DISTINCT caller_id)` without schema gymnastics. Per-row detail
(function name, per-node index, success flag, function args, return
value) moves to read-time XDR fetch through
`xdr_parser::extract_invocations`.

## Status: Completed

**Delivered:** 10/10 planned steps. ADR 0034 accepted. Rust gate green.
Handed off to task 0159 which cleans up the account-balances side next.

## Context

`soroban_invocations` stored one row per node in the Soroban
invocation tree — per ADR 0027 §10:

```
id, transaction_id, contract_id, caller_id, function_name, successful,
invocation_index, ledger_sequence, created_at
```

Every per-row column except identity/partitioning was already produced
by `xdr_parser::extract_invocations`. Applying the ADR 0033 pattern (DB
as pointer, heavy data from public archive XDR) closes the last
per-Soroban-tx detail leak out of the DB.

The single interesting table-specific question — `caller_id` retention
— was resolved during implementation through a cross-repo audit of
`unique_callers` consumers. See §Design Decisions → Emerged.

## Implementation

### Schema (migrations, in-place rewrite)

- `crates/db/migrations/0004_soroban_activity.sql` §10 replaced with
  `soroban_invocations_appearances (contract_id, transaction_id,
ledger_sequence, caller_id, amount, created_at)`. PK `(contract_id,
transaction_id, ledger_sequence, created_at)`. FKs preserved.
- `crates/db/migrations/20260421000100_replay_safe_uniques.up/down.sql`
  no longer carries `uq_soroban_invocations_tx_index` — the new PK covers
  replay idempotency.
- Two indexes: `idx_sia_contract_ledger (contract_id, ledger_sequence DESC)`
  for E13 list / E11 stats, `idx_sia_transaction (transaction_id)` for E3
  per-tx invocations lookup. `idx_inv_caller` from old schema dropped
  (no endpoint filters by caller).

### Indexer write path

- `crates/indexer/src/handler/persist/write.rs::insert_invocations`
  rewritten as aggregate: `HashMap<(contract_id, tx_id, ledger_sequence,
created_at), (amount, Option<caller_id>)>` with amount incremented per
  tree node and caller_id = first non-NULL (root-caller wins — parser
  emits depth-first so root is first-seen). `ON CONFLICT DO NOTHING` on
  composite PK.
- `type InvAggKey` / `type InvAggValue` aliases added to satisfy
  clippy::type_complexity.

### Staging cleanup

- `crates/indexer/src/handler/persist/staging.rs::InvRow` slimmed to
  identity fields only (`tx_hash_hex`, `contract_id`, `caller_str_key`,
  `ledger_sequence`, `created_at`). Removed `function_name`, `successful`,
  `invocation_index`. `caller_str_key` kept for write-time payload
  resolution.
- Flatten loop comment updated to document root-caller-wins depth-first
  ordering invariant.

### Domain cleanup

- `crates/domain/src/soroban.rs`: deleted `SorobanInvocation` struct
  (grep confirmed zero call sites). Module header updated to cover
  ADR 0033/0034 "DB index only, detail from archive" pattern for both
  events and invocations.

### Partition management + benches

- `crates/db-partition-mgmt/src/lib.rs`: `TIME_PARTITIONED_TABLES`
  renamed entry.
- `crates/backfill-bench/src/main.rs`: default-partition list renamed.

### Documentation

- **ADR 0034 drafted** (accepted) — precedent ADR 0033 linked; decision
  deliberately documents the `caller_id` divergence from 0033's
  "bare 4-column appearance" shape.
- **ADR 0021 coverage matrix** — E11 rewritten to use `SUM(amount)` +
  `COUNT(DISTINCT caller_id)` over appearances; E13 moved to "DB
  appearance + read-time XDR" with shared memoisation note pointing to
  E14's path. Catalog row 10 (schema table) + §6.4 tx-detail source row
  - §"Schema headroom" updated.
- **ADR 0027 §10** — superseded-for-this-table marker pointing to 0034.

### Tests

- `crates/indexer/tests/persist_integration.rs`: `Counts` gained
  `invocations_amount_sum: i64`. New CTE `ivs` summing `amount`. New
  assertion `SUM(amount) = ingested tree-node count` (1 for the
  fixture). Partition bootstrap loop + `test_counts` SELECT renamed to
  new table name.

### Related `xdr-parser` doc drifts cleaned up

- `types.rs::ExtractedInvocation` doc comment updated to reference the
  new table and depth-first emission invariant.
- `invocation.rs::InvocationResult.invocations` doc updated to
  reference ADR 0034 aggregation.

## Acceptance Criteria

- [x] `soroban_invocations_appearances` matches ADR 0034 §Decision.1
      DDL; `soroban_invocations` no longer exists in migrations or code.
- [x] `insert_invocations` aggregates per
      `(contract, tx, ledger, created_at)`; `persist_integration` asserts
      `SUM(amount)` equals the ingested tree-node count.
- [x] No compile-time reference to `function_name`, `successful`, or
      `invocation_index` for this table (verified by grep + green
      workspace build).
- [x] ADR 0021 coverage matrix updated (E11 / E13 rows + catalog row 10);
      ADR 0027 §10 carries a superseded-for-this-table marker; ADR 0034
      accepted.
- [x] `cargo build --workspace`, `cargo test --workspace --lib`,
      `cargo clippy --workspace --all-targets -- -D warnings`, and
      `cargo fmt --all -- --check` all green.
- [ ] Read-path wire-up (E11 stats, E13 handler) — **deferred**
      (API bootstrap out of scope; handled with the API follow-up that
      also lands E3 / E10 / E14 from 0157).

## Design Decisions

### From Plan

1. **Mirror 0157's 10-step sequence.** Deliberate symmetry — schema,
   write-path, staging, domain, partition-mgmt, ADR 0021 matrix,
   ADR 0027 marker, integration test, size-measurement baseline.
2. **Amount = tree-node count per (contract, tx, ledger) trio.**
   Matches events pattern; keeps row count the same as today's
   per-node storage (collapsed into a single aggregate row). OQ2
   resolved in §Decision.5 of ADR 0034.

### Emerged

3. **Keep `caller_id` as payload column — divergence from ADR 0033.**
   Discovered during the `unique_callers` audit: the frontend spec
   (`frontend-overview.md:431`, task 0075, ADR 0021 §E11) carries
   `unique_callers` as a visible stat. Dropping `caller_id` would
   either change the measured number (JOIN via `transactions.source_id`
   counts users reaching contract through wrappers — semantically
   different from today's filter behaviour) or require a separate
   materialisation.
   Kept as unindexed payload; `COUNT(DISTINCT caller_id)` preserves the
   existing staging filter semantics bit-for-bit. Documented explicitly
   in ADR 0034 §Decision.3.
4. **Root-caller-per-trio aggregation rule.** Parser emits depth-first
   (root before sub-invocations); staging already filters contract-
   callers to NULL via `is_strkey_account`. First-seen non-NULL caller
   in the aggregate HashMap wins — matches today's DB semantics exactly
   (root-level G-caller retained; sub-invocation contract-callers
   dropped to NULL). Edge case (single tx with multiple
   `InvokeHostFunctionOp`s targeting same contract with different root
   callers) documented in ADR 0034 §Decision.6 as "first-seen wins;
   real-world frequency vanishing".
5. **`idx_sia_caller` NOT added.** Original schema had
   `idx_inv_caller (caller_id, created_at DESC)` supporting a
   hypothetical "list invocations by caller" query. No endpoint in
   ADR 0021 uses this pattern (account-page lists transactions, not
   per-contract invocations). Adding the index later is non-breaking
   if the query ever materialises.
6. **Clippy `type_complexity` resolution.** The aggregation HashMap
   hit `clippy::type_complexity`. Resolved with `type InvAggKey =
(i64, i64, i64, DateTime<Utc>); type InvAggValue = (i64, Option<i64>);`
   aliases at the call site rather than `#[allow(...)]`.
7. **ADR 0021 catalog row 10 renamed, not removed.** Preserved row
   number 10 as "soroban_invocations_appearances" to keep the rest of
   the schema catalog numbering stable.
8. **API crate DTOs not touched.** Unlike 0157 which also trimmed
   `XdrEventDto`-related paths in `crates/api/src/stellar_archive/`,
   this task left `XdrInvocationDto` alone — those fields are exactly
   what read-time extraction feeds E13 and the DTO shape is correct
   as-is for when the handler lands.

## Issues Encountered

- **Clippy `type_complexity` on aggregation HashMap.** Initial signature
  `HashMap<(i64, i64, i64, DateTime<Utc>), (i64, Option<i64>)>` exceeded
  clippy's threshold. Fixed with `type` aliases per repo convention
  (already used in similar places in `write.rs`). Not a regression.
- **`lore/0-session/current-task.md` symlink retarget.** Required when
  promoting 0159 to current — session files are gitignored so re-creating
  the symlink doesn't generate a commit entry, only changes local state.

## Broken/modified tests

- `persist_integration.rs::persist_golden_ledger`:
  - `Counts` struct gained `invocations_amount_sum` field
  - New CTE `ivs` in `test_counts` SQL summing invocation amounts
  - New assertion: `invocations_amount_sum == 1` (one tree-node in the
    fixture's single invocation)
  - Partition bootstrap loop entry renamed `soroban_invocations` →
    `soroban_invocations_appearances`
  - `test_counts` SELECT joins renamed to new table name
  - Intentional test updates, not regressions. Replay test
    (`assert_eq!(counts_replay, counts_first, …)`) still covers
    idempotency.

## Future Work

- **Read-path wire-up for E11 stats + E13 handler** — deferred,
  bundled with 0157's deferred E3/E10/E14 handlers under the eventual
  API bootstrap task. No separate follow-up task spawned yet because
  the backlog entry for the API bootstrap (task 0046) already covers
  the contract page endpoint group.
- **Size-measurement baseline on full-sample indexer run** — the
  100-ledger sample showed mean-amount ≈ 1.001 (trees are mostly
  single-node in that window); bigger sample needed for meaningful
  projection. Captured as Open Question §1 in ADR 0034.

## Notes

- Bundled with task 0159 (drop `account_balance_history`) on the same
  PR branch because both were completed in the same session and are
  small docs+ADR deltas in addition to the code changes.
- ADR 0034 accepted immediately after implementation per project
  convention — status flow was `proposed` (drafted before code) →
  `accepted` (same day, after Rust gate green).
