---
id: '0169'
title: 'BUG: transactions.operation_count is wrong (sometimes hardcoded to 1 for multi-op tx, sometimes overcounted)'
type: BUG
status: backlog
related_adr: ['0037']
related_tasks: ['0167', '0168']
tags: [bug, indexer, xdr-parser, envelope-parsing, priority-high]
links: []
history:
  - date: 2026-04-27
    status: backlog
    who: fmazur
    note: 'Spawned from 0167 audit findings (Hypothesis B verification).'
---

# BUG: transactions.operation_count is wrong

## Summary

`transactions.operation_count` does not match the actual number of envelope operations as reported by Horizon. Two failure modes observed: (a) **undercount** — multi-op tx with Horizon op_count=6 or 3 are stored as `1` in our DB; (b) **overcount** — tx `cae5cb5747…` with Horizon op_count=1 is stored as `4` in our DB. The field appears to come from a wrong envelope walk path.

## Context

Audit during task **0167** sampled 6 tx from `ledger_sequence = 62016099`:

| App ord | Hash (prefix) | DB op_count | Horizon op_count |
| ------- | ------------- | ----------- | ---------------- |
| 0       | 90657bace9cf  | 1           | 1 ✓              |
| 50      | a6ed79663d64  | **1**       | **6** ❌         |
| 100     | dc98fb7f5f5a  | 1           | 1 ✓              |
| 150     | f6f95480d67b  | 1           | 1 ✓              |
| 200     | 64cc77ea0360  | **1**       | **3** ❌         |
| 255     | 358ef42d9840  | 1           | 1 ✓              |

Plus a separately-checked tx `cae5cb5747…` (app_order 242): DB op_count = **4**, Horizon op_count = **1**.

So the bug is asymmetric — undercounts wide multi-op tx, overcounts at least one specific case. This blocks both the §6.3 transactions list (renders the wrong ops badge) and the `idx_tx_has_soroban` path because the soroban flag derives from the same broken envelope walk.

## Implementation Plan

### Step 1: Locate the operation-count derivation

Search `crates/xdr-parser/src/` and `crates/indexer/src/handler/persist/` for where `operation_count` is computed. Probable misuses:

- Counting from a wrong array (e.g. inner tx ops vs outer fee-bump wrapper)
- Using a constant or `len()` on a non-array field
- Confusing `tx.operations.len()` with a downstream join

### Step 2: Verify against `tx.operations.len()` from the parsed envelope

The canonical source is the parsed `Transaction.operations` Vec from `xdr-parser`. After unwrapping the envelope variant correctly (see task **0168**), `operation_count = tx.operations.len() as i16` is the right shape.

### Step 3: Add unit tests

Cover txs with 1, 3, 6, 25 ops + a fee-bump'd inner tx with multiple ops. Confirm the count matches what Horizon would report for the same envelope.

### Step 4: Reindex

Same reindex strategy as 0168.

## Acceptance Criteria

- [ ] `operation_count` derives from `tx.operations.len()` of the correctly-unwrapped envelope.
- [ ] Unit tests cover 1-op, multi-op, and fee-bump'd multi-op cases.
- [ ] Re-running the indexer over the 6 sample tx + `cae5cb57…` produces `operation_count` matching Horizon 7/7.
- [ ] **Docs updated** — N/A.

## Notes

- Likely fixed in the same PR as **0168** because both stem from broken envelope-variant matching. Decision on scope split is the implementer's call.
- This bug also degrades the `idx_tx_has_soroban` partial index because `has_soroban` is similarly broken (see **0170**) — many list endpoints are unreliable until all three are fixed.
