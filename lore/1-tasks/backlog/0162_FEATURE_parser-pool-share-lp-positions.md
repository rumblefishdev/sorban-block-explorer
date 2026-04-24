---
id: '0162'
title: 'Parser: emit pool_share trustlines as ExtractedLpPosition rows'
type: FEATURE
status: backlog
related_adr: ['0027']
related_tasks: ['0126', '0136']
tags: [priority-low, effort-small, layer-xdr-parser, audit-gap]
milestone: 1
links:
  - crates/xdr-parser/src/state.rs
  - crates/xdr-parser/src/types.rs
  - crates/indexer/src/handler/persist/mod.rs
  - crates/indexer/src/handler/process.rs
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
---

# Parser: emit pool_share trustlines as ExtractedLpPosition rows

## Summary

`xdr_parser::extract_account_states` currently drops every trustline
with `asset.type == "pool_share"` (`state.rs:231-234`). These
trustlines encode LP positions: `(account, pool_id, share balance)`.
Without them, `persist_ledger` receives an empty `lp_positions` slice
every invocation and the `lp_positions` table stays empty — regardless
of how well task 0126 wires the persist / API layer downstream.

This task is the parser-level prereq for 0126. Scope is narrow: just
emit the data. Schema extensions, persist wiring, and API surface
belong to 0126.

## Context

Today in `crates/xdr-parser/src/state.rs:225-246`:

```rust
let trustline_entry = match asset {
    Some(Value::Object(obj)) => {
        let asset_type = obj.get("type")...;
        // Skip pool_share trustlines — LP positions, not asset balances
        if asset_type == "pool_share" {
            continue;
        }
        ...
    }
    _ => continue,
};
```

Downstream already has the wiring:

- `ExtractedLpPosition` type exists in `xdr-parser/src/types.rs:347`.
- `persist_ledger` accepts `&[ExtractedLpPosition]`
  (`persist/mod.rs:101`).
- `Staged::prepare` converts to `LpPositionRow` at
  `staging.rs:706-716`.
- `crates/indexer/src/handler/process.rs:163` hardcodes
  `let lp_positions: Vec<ExtractedLpPosition> = Vec::new();`.

Parser is the only missing piece.

## Implementation

### 1. Emit from parser

Add `extract_lp_positions(changes: &[ExtractedLedgerEntryChange])
-> Vec<ExtractedLpPosition>` to `xdr-parser/src/state.rs` (sibling to
`extract_account_states` / `extract_liquidity_pools`). For each
trustline change with `asset.type == "pool_share"`:

- `pool_id` = hex-encoded `asset.pool_id` (check XDR shape; may live
  under `liquidity_pool` key).
- `account_id` = trustline's `account_id`.
- `shares` = `balance` as string (stroops, same format as other
  balances).
- `last_updated_ledger` = `change.ledger_sequence`.
- `first_deposit_ledger` = `Some(ledger_sequence)` on `created`
  change_type, else `None` (staging preserves existing via COALESCE).

Don't skip pool_share in `extract_account_states`; instead route to
the new fn. Either call separately in `process.rs` or return both
vectors from a combined fn — match the existing idiom (separate fn
per output type is the pattern today).

### 2. Wire in process.rs

Replace `persist/process.rs:163`:

```rust
let lp_positions: Vec<ExtractedLpPosition> = Vec::new();
```

with per-tx accumulation like other `extract_*` calls:

```rust
let mut all_lp_positions = Vec::new();
// inside the ledger_entry_changes loop:
let lp_pos = xdr_parser::extract_lp_positions(changes);
all_lp_positions.extend(lp_pos);
// pass &all_lp_positions into persist_ledger
```

### 3. Tests

- **Unit** (`state.rs`): `pool_share` trustline with shares = 42
  → `ExtractedLpPosition { pool_id, account_id, shares: "42", .. }`.
- **Unit**: regular credit trustline unchanged — goes to
  `extract_account_states`, not `extract_lp_positions`.
- **Unit**: `removed` change_type for pool_share → decide: emit with
  `shares = "0"` (persist layer decides prune policy) or skip (0126
  handles removal separately). Document the choice in this task.
- **Integration** (`persist_integration.rs`): synthetic ledger with
  one pool_share trustline create → `lp_positions` table has one row
  after persist. (Schema for the row is pre-existing from migration
  0006 §16.)

## Acceptance Criteria

- [ ] `xdr_parser::extract_lp_positions` exported, covers pool_share
      trustline extraction from ledger entry changes.
- [ ] `process.rs` wires the parser output into `persist_ledger`.
- [ ] Existing `extract_account_states` no longer silently drops
      pool_share entries in a way that loses data (routes to new fn,
      or delegates).
- [ ] Unit + integration coverage per §3.
- [ ] Deletion semantics (removed trustline → decrease / zero /
      delete) documented and implemented consistently.
- [ ] Does NOT regress account_balances_current writes (regular
      credit trustlines stay on the existing path).

## Notes

Audit gap. Unblocks 0126. Effort small because scaffolding end-to-end
already exists — this is purely the missing producer.
