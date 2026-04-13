---
id: '0119'
title: 'Indexer: extract trustline balances for accounts'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0027', '0048']
tags: [priority-high, effort-medium, layer-indexer, audit-F7]
milestone: 1
links:
  - crates/xdr-parser/src/state.rs
  - docs/audits/2026-04-10-pipeline-data-audit.md
history:
  - date: '2026-04-10'
    status: backlog
    who: stkrolikiewicz
    note: 'Spawned from pipeline audit finding F7 (HIGH severity).'
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

- [ ] `balances` JSONB contains native XLM + all trustline balances
- [ ] Trustline format: `{"asset_type": "credit_alphanum4", "asset_code": "USDC", "issuer": "G...", "balance": "100.0000000"}`
- [ ] Trustline removal correctly removes entry from balances array
- [ ] Watermark prevents stale trustline data from overwriting newer state
- [ ] Tests: account with native + 2 trustlines produces correct balances array

## Out of Scope (2026-04-13)

**Contract token balances** (`contract_data` entries for Soroban tokens) are NOT covered by
this task. F7 in the audit mentions both trustline and contract token balances, but contract
token balance extraction depends on task 0120 (soroban-native token detection) and requires
parsing per-contract storage layouts. This should be a separate follow-up task once 0120
lands.
