---
id: '0138'
title: 'Indexer: extract contract token balances from contract_data entries'
type: FEATURE
status: backlog
related_adr: ['0012']
related_tasks: ['0119', '0120', '0135', '0140', '0142']
blocked_by: ['0120', '0142']
tags:
  [
    priority-high,
    effort-large,
    layer-indexer,
    layer-db,
    audit-F7,
    pending-adr-0012-rewrite,
  ]
milestone: 1
links:
  - docs/audits/2026-04-10-pipeline-data-audit.md
history:
  - date: '2026-04-15'
    status: backlog
    who: FilipDz
    note: 'Spawned from 0119 out-of-scope. Audit finding F7 covers both trustline and contract token balances; 0119 handled trustlines, this handles Soroban tokens.'
  - date: '2026-04-17'
    status: backlog
    who: stkrolikiewicz
    note: >
      Audit per task 0140 — ADR 0012 supersedes the underlying schema/flow
      patterns referenced in body. Blocked by 0142 (schema migration). Body
      must be re-read against ADR 0012 before implementing.
---

> **⚠ Post-ADR 0012 re-read required (audit 2026-04-17, [task 0140](../active/0140_DOCS_audit-lore-tasks-adr-0011-0012.md)):**
> Body below references pre-ADR-0012 patterns (schema / flow / JSONB / upsert / `transaction_id` partitioning). [ADR 0012](../../2-adrs/0012_zero-upsert-schema-full-fk-graph.md) supersedes with zero-upsert history tables, activity projections, S3 offload, and `created_at` partitioning. Blocked by 0142 (schema migration) — do not implement until migration lands and this task is re-aligned.

---

# Indexer: extract contract token balances from contract_data entries

## Summary

Task 0119 added trustline balance extraction for classic Stellar assets (credit_alphanum4/12).
Soroban token balances live in `contract_data` ledger entries, not trustlines, and require
per-contract storage layout parsing to extract. This task completes audit finding F7 by
adding contract token balances to the account `balances` JSONB array.

## Context

Soroban tokens (SEP-0041 compliant) store balances as `ContractData` entries keyed by
`(contract_id, Balance, address)`. Unlike trustlines which have a fixed XDR schema, contract
storage layouts vary — task 0120 (soroban-native token detection) must land first to identify
which contracts are tokens and how to parse their storage.

Once 0120 provides token contract detection, this task can extract balance values from
`contract_data` changes and merge them into the account's `balances` array alongside native
XLM and trustline balances.

## Implementation

1. Depend on task 0120's token contract registry to identify which `contract_data` entries
   represent token balances.
2. Parse balance values from `contract_data` entries (key structure: `Balance` + account address).
3. Associate extracted balances with the parent account.
4. Merge into the account's `balances` JSONB array using the existing JSONB merge SQL from 0119.
5. Handle balance creation, update, and removal (contract_data deletion).
6. Format: `{"asset_type": "contract", "contract_id": "C...", "balance": "X.XXXXXXX"}`.
7. Decide on decimal precision: Soroban tokens define their own `decimals()` (could be 6, 7, 8,
   18, etc.) — unlike native XLM which is always 7. Store raw i128 as string? Or normalize
   using the token's declared decimals? Needs design decision.

## Acceptance Criteria

- [ ] `balances` JSONB contains contract token balances alongside native + trustline balances
- [ ] Contract balance format: `{"asset_type": "contract", "contract_id": "C...", "balance": "X.XXXXXXX"}`
- [ ] Balance removal on contract_data deletion
- [ ] Watermark prevents stale contract data from overwriting newer state
- [ ] Tests: account with native + trustline + contract token produces correct balances array

## Open Questions

1. **Decimal precision**: Native XLM and trustlines always use 7 decimal places (stroops).
   Soroban tokens define arbitrary `decimals()`. Should we store the raw value as a string
   and include a `decimals` field, or normalize to the token's declared precision?
2. **Backfill ordering**: During parallel backfill, `contract_data` entries may be processed
   before 0120's token registry identifies the contract as a token. Options:
   a) Require 0120's registry to be pre-populated before backfill.
   b) Post-backfill reparse pass for contract_data entries.
   c) Inline check against `wasm_interface_metadata` at parse time (may miss if WASM not yet
   processed by that worker).
   Must be resolved before implementation — see guideline #4 (parallel workers) and #5
   (post-processing documentation).
