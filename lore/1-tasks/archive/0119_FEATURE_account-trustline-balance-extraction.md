---
id: '0119'
title: 'Indexer: extract trustline balances for accounts'
type: FEATURE
status: completed
related_adr: ['0012']
related_tasks: ['0027', '0048', '0140']
tags:
  [
    priority-high,
    effort-medium,
    layer-indexer,
    audit-F7,
    superseded-by-adr-0012,
  ]
milestone: 1
links:
  - crates/xdr-parser/src/state.rs
  - docs/audits/2026-04-10-pipeline-data-audit.md
history:
  - date: '2026-04-10'
    status: backlog
    who: stkrolikiewicz
    note: 'Spawned from pipeline audit finding F7 (HIGH severity).'
  - date: '2026-04-13'
    status: active
    who: FilipDz
    note: 'Activated for implementation'
  - date: '2026-04-15'
    status: done
    who: FilipDz
    note: >
      Implemented trustline balance extraction across 4 files (+758 lines).
      6 unit tests + 3 integration tests added. Key decisions: -1 sentinel
      for sequence_number on trustline-only updates, JSONB merge SQL for
      concurrent-safe balance array updates, pool_share trustlines skipped.
  - date: '2026-04-17'
    status: done
    who: stkrolikiewicz
    note: >
      Flagged per task 0140 audit — implementation pattern superseded by ADR 0012
      (zero-upsert schema, insert-only history tables, S3 offload, activity
      projections, created_at partitioning). Body retained as historical record;
      do not use as reference for new work — consult ADR 0012 instead.
---

# Indexer: extract trustline balances for accounts

## Summary

`extract_account_states()` currently hardcodes a single native XLM balance. Trustline
balances (credit_alphanum4, credit_alphanum12) from `TrustLineEntry` LedgerEntry types
are never processed. The `balances` JSONB column was designed for a multi-balance array
but always contains `[{"asset_type": "native", "balance": X}]`.

## Context

The account detail page (task 0048/0073) needs to show all account balances — native XLM
plus all trustline positions. Without this, the explorer's account page is significantly
incomplete compared to competitors (StellarExpert, Stellarchain).

## Implementation

1. Process `trustline` entry type in `extract_ledger_entry_changes()` — extract asset code,
   issuer, balance, and limit.
2. Associate trustline entries with their parent account (trustline key contains account_id).
3. Merge trustline balances into the account's `balances` JSONB array alongside native XLM.
4. Handle trustline creation, update, and removal (deletion = balance removed from array).
5. Watermark logic: trustline updates should trigger account `last_seen_ledger` update.

## Acceptance Criteria

- [x] `balances` JSONB contains native XLM + all trustline balances
- [x] Trustline format: `{"asset_type": "credit_alphanum4", "asset_code": "USDC", "issuer": "G...", "balance": "0.0005000"}`
- [x] Trustline removal correctly removes entry from balances array
- [x] Watermark prevents stale trustline data from overwriting newer state
- [x] Tests: account with native + 2 trustlines produces correct balances array

## Implementation Notes

**Files changed (4, +758 lines):**

- `crates/xdr-parser/src/state.rs` — Two-pass `extract_account_states()`: pass 1 processes account entries (native balance, seq_num, home_domain), pass 2 processes trustline entries (credit_alphanum4/12). Merges by account_id via HashMap accumulator. Added `format_stroops()` for i64→decimal string conversion.
- `crates/xdr-parser/src/types.rs` — Added `removed_trustlines: Vec<Value>` to `ExtractedAccountState`.
- `crates/db/src/soroban.rs` — Upsert SQL uses JSONB merge (keeps existing balances not in EXCLUDED, adds new). `sequence_number` CASE guard for -1 sentinel. New `remove_trustlines_batch()` with watermark guard.
- `crates/indexer/src/handler/persist.rs` — Dedup changed from last-wins to merge logic (combines balances, sequence_number, home_domain, removed_trustlines). Removal batch runs after upsert.

**Tests added (9):**

- Unit: `account_with_two_trustlines`, `trustline_only_change`, `trustline_removal`, `trustline_update_dedup`, `pool_share_trustline_skipped`, `removal_cancels_same_tx_creation`
- Integration: `trustline_upsert_preserves_native`, `remove_trustlines_batch_removes_entry`, `watermark_with_jsonb_merge`

## Design Decisions

### From Plan

1. **Trustline entries merged into account's balances array**: As specified — trustline key contains account_id, used to associate with parent account.
2. **Separate removal tracking**: `removed_trustlines` vec avoids polluting the balances array with deletion markers.

### Emerged

3. **-1 sentinel for sequence_number**: Trustline-only changes (no account entry in same tx) produce `sequence_number = -1`. SQL CASE guard preserves existing value. Alternative was `Option<i64>` but would require schema change across 3 crates.
4. **pool_share trustlines skipped**: LP share positions are not token balances — they belong in the liquidity pool domain (task 0126). Silently filtered out.
5. **format_stroops decimal strings**: Balances stored as `"0.1000000"` (7 decimal places) matching Stellar's stroop precision, not raw i64. Consistent with Horizon API format.
6. **JSONB merge SQL over full replacement**: ON CONFLICT uses subquery to merge arrays element-by-element (match by asset_type+asset_code+issuer). Allows trustline-only updates without losing native balance.
7. **Watermark on remove_trustlines_batch**: Added during code review — original implementation had no ledger guard, allowing stale replays to strip trustlines.

## Known Limitations

- **Parallel worker ordering**: If worker B processes a trustline-only change before worker A processes the account creation, the INSERT uses the sentinel `sequence_number = -1` which the DB stores as-is (no existing row to CASE against). Corrected automatically when the account entry is processed by any worker — the upsert overwrites with the real sequence number. Acceptable for backfill; no data loss.
- **No schema migration needed**: The `balances` column was already `jsonb` designed for a multi-element array. No DDL changes required.

## Out of Scope (2026-04-13)

**Contract token balances** (`contract_data` entries for Soroban tokens) are NOT covered by
this task. F7 in the audit mentions both trustline and contract token balances, but contract
token balance extraction depends on task 0120 (soroban-native token detection) and requires
parsing per-contract storage layouts. This should be a separate follow-up task once 0120
lands.
