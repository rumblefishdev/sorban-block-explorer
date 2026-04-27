---
id: '0168'
title: 'BUG: transactions.source_id mis-extracted from envelope variants (FeeBump unwrap + likely V0/V1 mismatch)'
type: BUG
status: active
related_adr: ['0037', '0026']
related_tasks: ['0167']
tags: [bug, indexer, xdr-parser, envelope-parsing, priority-critical]
links: []
history:
  - date: 2026-04-27
    status: backlog
    who: fmazur
    note: 'Spawned from 0167 audit findings (Hypothesis B verification). Subagent compared 6 random tx vs Horizon mainnet — 6/6 source mismatch.'
  - date: 2026-04-27
    status: active
    who: fmazur
    note: 'Promoted to active via /promote-task.'
---

# BUG: transactions.source_id mis-extracted from envelope variants

## Summary

The indexer writes the **wrong source account** to `transactions.source_id` for every transaction sampled (6/6 in the audit). Worst case, observed on a fee-bump transaction: the DB stores the **fee-bump fee_account** instead of the **inner tx source**. For non-fee-bump txs the source is still wrong but in a different way — pointing at unrelated accounts entirely. Hash + ledger + closed_at parsed correctly; only the envelope-derived source is broken.

## Context

Audit during task **0167** (endpoint SQL query reference set) checked 6 random tx from `ledger_sequence = 62016099` (application_orders 0, 50, 100, 150, 200, 255) against Horizon mainnet. Every single source account mismatched. Tx at app_order 255 (`358ef42d9840…`) is the smoking gun: DB stores `GA2JRQOF…HVAW`, which equals Horizon's `fee_account` (the fee-bump payer), NOT the inner `source_account` (`GCZYOCHU…MCQS`). For the other 5 tx (which are not fee-bump'd) the DB source is also wrong but the pattern isn't yet diagnosed — likely a wrong envelope-variant match arm in `xdr-parser`.

This breaks every list / detail endpoint that surfaces or filters by source: E2 (transactions list), E7 (account transactions), E22 (search), and the `source_account` column rendered everywhere on the frontend.

## Implementation Plan

### Step 1: Locate the responsible code

Subagent investigation pointed to `crates/xdr-parser/src/transaction.rs`. The hash is correctly extracted from `TransactionResultPair` (line 99 comment: "avoids needing network_id"), but the source extraction goes through envelope variant matching that's broken. Search for `TransactionEnvelope::` match arms and check coverage of:

- `TransactionEnvelope::Tx(TransactionV1Envelope)` — `tx.source_account`
- `TransactionEnvelope::TxV0(TransactionV0Envelope)` — `tx.source_account_ed25519` (NOT the same shape; bare ed25519 pk, must be wrapped into `MuxedAccount::Ed25519`)
- `TransactionEnvelope::TxFeeBump(FeeBumpTransactionEnvelope)` — must descend into `tx.inner_tx.tx.source_account`, NOT `tx.fee_source`

### Step 2: Fix the match arms

Implement complete envelope-variant unwrapping. The fee-bump case in particular needs nested unwrapping back to the inner transaction's source. Add unit tests covering all three variants explicitly with hand-crafted XDR fixtures.

### Step 3: Reindex affected data

Once fixed, the entire historical backfill needs re-running (or an in-place UPDATE driven by re-parsing archive XDR) — because every existing `transactions.source_id` is suspect.

## Acceptance Criteria

- [ ] Unit tests in `crates/xdr-parser/tests/` cover all three `TransactionEnvelope` variants (`Tx`, `TxV0`, `TxFeeBump`) with hand-crafted fixtures asserting the correct `source_account` is extracted in each case.
- [ ] Re-running the indexer over the same 6 sample tx (hashes captured in 0167 audit) produces source accounts that match Horizon 6/6.
- [ ] Documented strategy for reindexing existing rows (full DB wipe + backfill, or in-place UPDATE driven by archive XDR re-parse).
- [ ] **Docs updated** — N/A — no shape change to schema or API surface; ADR 0037 unchanged.

## Notes

- This bug also explains the apparent inconsistency in 0167 task notes about `has_soroban=t` showing up on classic offers — every assumption built on top of `transactions.source_id` was suspect.
- Sister bugs from the same audit: **0169** (`operation_count` wrong) and **0170** (`has_soroban` wrong). All three live in the same envelope-parsing code path. Probably one fix PR can address all three; alternatively three separate PRs for cleaner review.
- Subagent transcript with sample comparison table is in 0167's task history (date 2026-04-27).
