---
id: '0170'
title: 'BUG: transactions.has_soroban over-set (always true on sample, breaks Soroban filter and badge)'
type: BUG
status: backlog
related_adr: ['0037']
related_tasks: ['0167', '0168', '0169']
tags: [bug, indexer, xdr-parser, envelope-parsing, priority-high]
links: []
history:
  - date: 2026-04-27
    status: backlog
    who: fmazur
    note: 'Spawned from 0167 audit findings (Hypothesis B verification).'
---

# BUG: transactions.has_soroban over-set

## Summary

`transactions.has_soroban` is `true` on every transaction sampled (6/6 + the one previously hand-checked `cae5cb5747…`), even though only **one** of those tx actually carries an `INVOKE_HOST_FUNCTION` op (the rest are classic offers / payments / SetOptions). The flag looks hardcoded or read from a wrapper that is always present. This breaks the partial index `idx_tx_has_soroban ON (created_at DESC) WHERE has_soroban` and any frontend Soroban badge.

## Context

Audit during task **0167** picked 6 random tx from ledger 62016099:

| App ord | DB has_soroban | Horizon op type                | Should be |
| ------- | -------------- | ------------------------------ | --------- |
| 0       | t              | path_payment_strict_send       | f         |
| 50      | t              | manage\_\*\_offer (×6)         | f         |
| 100     | t              | path_payment_strict_receive    | f         |
| 150     | t              | manage_sell_offer              | f         |
| 200     | t              | manage_sell_offer (×3)         | f         |
| 255     | t              | invoke_host_function "harvest" | t ✓       |

5/6 should be `false`. Only the actual Soroban tx is correctly `true`, but that's the trivial case (always true).

Open question for the implementer: **what semantics do we want?**

- **Strict (Stellar protocol)**: `has_soroban = operations.iter().any(|op| matches!(op.body, InvokeHostFunction(_) | ExtendFootprintTtl(_) | RestoreFootprint(_)))`
- **Loose (event emission)**: `has_soroban = true` whenever any soroban event fires in the tx, including SAC-side `transfer` events emitted by classic ops on SAC-backed assets (HELIX, USDC, EURC, etc.)

The loose interpretation has UX value (it tells the user "this tx interacted with the Soroban side of things even if classically authored"), but the field name suggests strict. Pick one, document it in ADR 0037 §5 (transactions table notes), and stick to it.

## Implementation Plan

### Step 1: Decide semantics

Recommend **strict** to match the field name and the partial-index intent. Anything looser belongs in a separate column (e.g. `has_soroban_event`) so the frontend can distinguish "this tx ran soroban code" from "soroban-side event emitted as a side effect of classic ops".

### Step 2: Locate the derivation

Search `crates/xdr-parser/src/transaction.rs` for `has_soroban`. Likely candidates:

- Reading `Some(soroban_data)` on a struct that always exists post-protocol-20
- Hardcoded `true`
- Any non-`operations.iter().any(...)` derivation

### Step 3: Implement the strict check

```rust
let has_soroban = parsed_ops.iter().any(|op| matches!(
    op.body,
    OperationBody::InvokeHostFunction(_)
        | OperationBody::ExtendFootprintTtl(_)
        | OperationBody::RestoreFootprint(_)
));
```

### Step 4: Reindex

Same reindex as 0168.

## Acceptance Criteria

- [ ] `has_soroban = true` iff at least one of `INVOKE_HOST_FUNCTION` / `EXTEND_FOOTPRINT_TTL` / `RESTORE_FOOTPRINT` op present in envelope.
- [ ] Unit tests: classic-only tx → `false`; Soroban-only tx → `true`; mixed (impossible per Stellar protocol but defensive) → `true`; fee-bump'd Soroban → `true` after correct envelope unwrap.
- [ ] Re-running the indexer over the 6 sample tx produces the right `has_soroban` 6/6.
- [ ] If we ever want the loose interpretation, it lives in a separate boolean column with a name making the difference obvious (`has_soroban_event` or similar).
- [ ] **Docs updated** — ADR 0037 §5 (transactions table) gains one-line note on `has_soroban` semantics if the strict-vs-loose decision is documented at the schema level.

## Notes

- Sister bugs from the same audit: **0168** (source_id wrong) and **0169** (operation_count wrong). All three are envelope-parsing defects in the same crate.
- After the fix, `idx_tx_has_soroban` becomes useful again (currently it's a useless index because every row matches its WHERE clause).
