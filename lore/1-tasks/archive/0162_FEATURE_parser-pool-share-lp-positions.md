---
id: '0162'
title: 'Parser: emit pool_share trustlines as ExtractedLpPosition rows'
type: FEATURE
status: completed
related_adr: ['0024', '0037']
related_tasks: ['0126', '0136']
tags: [priority-low, effort-small, layer-xdr-parser, audit-gap]
milestone: 1
links:
  - crates/xdr-parser/src/state.rs
  - crates/xdr-parser/src/lib.rs
  - crates/indexer/src/handler/process.rs
  - crates/indexer/tests/persist_integration.rs
history:
  - date: '2026-04-24'
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned to unblock 0126. Real prerequisite for pool participants
      tracking is parser work emitting pool_share trustlines as
      ExtractedLpPosition rows — skipped today at
      crates/xdr-parser/src/state.rs:231-234. 0136 (superseded) was
      the formal blocker but it's void; this task captures the actual
      parser gap.
  - date: '2026-04-27'
    status: active
    who: stkrolikiewicz
    note: >
      Promoted. Closes the third (and last) audit-gap from the
      2026-04-10 pipeline data audit (0160 SAC identity + 0161 native
      singleton + 0162 LP positions parser). Narrow parser-only scope:
      emit pool_share trustlines as ExtractedLpPosition rows; persist /
      API layer is 0126's job.
  - date: '2026-04-27'
    status: completed
    who: stkrolikiewicz
    note: >
      Shipped on `feat/0162_parser-pool-share-lp-positions` (single
      commit). New `xdr_parser::extract_lp_positions` consumes the
      pool_share trustline changes that `extract_account_states` skips,
      emits one ExtractedLpPosition per change with stroop-formatted
      shares and `first_deposit_ledger` set only on `created`. Wired
      into `process.rs` per-tx accumulation; `persist_ledger` now
      receives populated lp_positions slice. 5 new state.rs unit tests
      + extended `synthetic_ledger_insert_and_replay_is_idempotent`
      with a real LP position assertion. All 162 xdr-parser unit + 9/9
      persist_integration parallel pass; clippy clean. Removal semantic
      chosen: emit zero-shares row (decision #2 below).
---

# Parser: emit pool_share trustlines as ExtractedLpPosition rows

## Summary

`xdr_parser::extract_account_states` was skipping every trustline with
`asset.type == "pool_share"` (`state.rs:256-259` + `:321-324`),
silently dropping LP participant data. Schema, types, persist, and
staging layers all already had wiring for `ExtractedLpPosition` —
parser was the only missing producer. This task fills that gap with a
narrow sibling fn `extract_lp_positions` that consumes the same
`changes` slice and emits one position per pool_share trustline
change. Persist + API layer remain task 0126's responsibility.

## Context

Pre-fix flow:

- `state.rs:256-259` (created/updated/restored branch) — `if
asset_type == "pool_share" { continue; }`
- `state.rs:321-324` (removed branch) — same skip
- `process.rs:212` — hardcoded `let lp_positions: Vec<...> = Vec::new();`
- `lp_positions` table stayed empty across every reindex

Downstream wiring already in place:

- `ExtractedLpPosition` in `types.rs:388`
- `persist_ledger` accepts `&[ExtractedLpPosition]`
  (`persist/mod.rs:101`)
- `Staged::prepare` → `LpPositionRow` at `staging.rs:768-778` with
  `decode_hash(&pos.pool_id, ...)` + COALESCE-style update path

## Implementation

One commit on `feat/0162_parser-pool-share-lp-positions`:

1. **`xdr_parser::extract_lp_positions`** in `state.rs` (sibling to
   `extract_liquidity_pools`): iterate `changes`, filter `entry_type
== "trustline"`, extract `(pool_id, account_id, balance)` from
   `data` (created/updated/restored) or `key` (removed), emit
   `ExtractedLpPosition` with `format_stroops(balance)` shares and
   `first_deposit_ledger = Some(seq)` on `created` only. Skips
   `state` change_type (observation, no balance change). Pre-existing
   `pool_share` skips in `extract_account_states` retained — the data
   is no longer lost because `process.rs` now calls both producers
   on the same `changes` slice.
2. **`process.rs` wiring**: new `all_lp_positions` accumulator,
   per-tx `xdr_parser::extract_lp_positions(changes)` call inside
   the existing ledger-entry-changes loop, `&all_lp_positions`
   passed into `persist::persist_ledger` (replaces the
   `Vec::new()` placeholder from task 0149).
3. **lib.rs** re-export.
4. **Tests**: 5 new unit in `state.rs::tests` (created /
   updated / removed / credit-trustline-ignored /
   state-change-type-ignored). Integration:
   `synthetic_ledger_insert_and_replay_is_idempotent` extended with
   one synthetic LP position on the existing `POOL_ID` fixture; old
   `assert_eq!(counts.lp_positions, 0)` flips to `assert_eq!(=, 1)`.
   Replay still asserts idempotency via `counts_replay == counts_first`.

## Acceptance Criteria

- [x] `xdr_parser::extract_lp_positions` exported, covers pool_share
      trustline extraction from ledger entry changes.
- [x] `process.rs` wires the parser output into `persist_ledger`.
- [x] `extract_account_states` no longer loses pool_share data — the
      skip remains, but `process.rs` now invokes both producers on
      the same changes slice (route-via-separate-fn pattern).
- [x] Unit (5 in `state.rs::tests`) + integration (extended
      `synthetic_ledger_insert_and_replay_is_idempotent`) coverage.
- [x] Removal semantics implemented and documented (decision #2
      below).
- [x] Does NOT regress `account_balances_current` writes — the
      credit-trustline path in `extract_account_states` is untouched;
      `lp_positions_ignore_credit_trustlines` test pins this.

## Implementation Notes

| File                                          | Δ                                                 |
| --------------------------------------------- | ------------------------------------------------- |
| `crates/xdr-parser/src/state.rs`              | +97 producer + 5 unit tests                       |
| `crates/xdr-parser/src/lib.rs`                | +1 (re-export)                                    |
| `crates/indexer/src/handler/process.rs`       | +9/-3 (accumulator + per-tx call + arg swap)      |
| `crates/indexer/tests/persist_integration.rs` | +13/-3 (LP fixture + flipped assertion + comment) |

**Tests**: 162 xdr-parser unit (+5 new), 9/9 persist_integration parallel.
Clippy `--workspace --all-targets -- -D warnings` clean.

**Migrations**: none. Schema for `lp_positions` table pre-exists from
migration `0006_liquidity_pools.sql`.

## Issues Encountered

None. Scaffolding end-to-end already existed (per task notes); this
was purely the missing producer.

## Design Decisions

### From Plan

1. **Separate sibling fn over modifying `extract_account_states` to
   emit both outputs.** Matches the existing one-fn-per-output-type
   idiom in `state.rs` (`extract_account_states`,
   `extract_liquidity_pools`, `extract_contract_deployments`,
   `detect_assets`, `detect_nfts`). Two passes over `changes` cost
   ~zero (changes bounded by tx size).

### Emerged

2. **`removed` change_type emits zero-shares row, does NOT skip.**
   Task notes left the call to implementation. Picked emit-with-zero
   because:

   - Persist layer (task 0126) gets to choose the prune policy —
     keep historical participant rows with `shares = 0`, or DELETE
     when seen in this shape. Either is valid, neither belongs to
     parser.
   - Skipping at the parser would lose the change-watermark —
     downstream couldn't distinguish "never deposited" from "deposited
     and withdrew".
   - Symmetric with how `extract_account_states` reports trustline
     removal (it does emit a `removed_trustlines` entry, not silently
     drop).

3. **`state` change_type ignored.** Same rationale as the trustline
   path in `extract_account_states` — `state` is observation-only with
   no balance delta. Unit test `lp_positions_ignore_state_change_type`
   pins this.

4. **`first_deposit_ledger` only set on `created`.** `updated` /
   `restored` / `removed` emit `None`. Staging layer COALESCEs to
   preserve the original first-deposit ledger across subsequent
   updates. Matches the comment in `types.rs:395-397` ("Set only on
   the first appearance of `(pool_id, account_id)`; `None` on
   subsequent updates").

5. **Pre-existing `pool_share` skips in `extract_account_states`
   retained.** Removing them would cause pool_share entries to land
   in the per-account `trustline_balances` JSON, which is shaped for
   classic credit balances (asset_code + issuer fields). The skip is
   semantically correct ("this fn doesn't handle pool_share"); the
   data is no longer lost because there's now a sibling fn that does
   handle them.

## Future Work

None for parser scope. Task 0126 (LP participant tracking) is the
direct downstream consumer — it owns:

- API surface for `/liquidity-pools/:pool_id/positions`
- Frontend integration
- Decision on prune policy for `shares = 0` rows (keep vs delete)
- Possibly: derive total participant count per pool from `lp_positions`
  for pool detail page

## Notes

Audit gap. Closes the third (and last) of the three 2026-04-10
pipeline-data-audit gaps: 0160 (SAC identity), 0161 (native
singleton), 0162 (LP positions parser). After 0126 lands the
downstream consumer, the audit is fully zaorane.
