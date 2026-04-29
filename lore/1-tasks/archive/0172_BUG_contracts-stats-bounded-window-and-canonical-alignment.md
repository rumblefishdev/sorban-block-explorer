---
id: '0172'
title: 'BUG: contracts E10 stats — bounded window + correct metric + missing E10 fields per task 0167 audit'
type: BUG
status: completed
related_adr: ['0008', '0030', '0031', '0034', '0037']
related_tasks: ['0050', '0132', '0167']
tags: [priority-high, layer-backend, contracts, audit-driven]
milestone: 2
links:
  - https://github.com/rumblefishdev/soroban-block-explorer/pull/126
  - docs/architecture/database-schema/endpoint-queries/11_get_contracts_by_id.sql
history:
  - date: '2026-04-28'
    status: backlog
    who: FilipDz
    note: >
      Spawned from task 0167 audit on PR #126. Canonical SQL
      `11_get_contracts_by_id.sql` landed 8 min before 0050 merged so the
      divergences are a historical artifact. HIGH: `fetch_contract_stats`
      full-history scans both partitioned appearance tables with
      `SUM(amount)` instead of canonical's bounded
      `COUNT(*) + COUNT(DISTINCT caller_id)`.
  - date: '2026-04-28'
    status: active
    who: FilipDz
    note: >
      Shipped on `fix/0172_contracts-stats-bounded-window`: full canonical
      alignment for E10/E11/E13/E14 plus `crate::common::*` migration
      (task 0043). Details in the AC list. 80/80 live tests pass;
      clippy `-D warnings` clean.
---

# BUG: contracts E10/E11/E13/E14 — canonical SQL alignment + bounded stats

## Summary

Align `crates/api/src/contracts/` with the canonical SQL deliverable from
task 0167 (`docs/architecture/database-schema/endpoint-queries/{11..14}_*.sql`).
The audit on PR #126 flagged one HIGH (unbounded `fetch_contract_stats`)
plus several LOW divergences; all four endpoints sit in one module so
the LOW ones are bundled in the same PR.

## Acceptance Criteria

- [x] `fetch_contract_stats` queries `soroban_invocations_appearances` only;
      uses `COUNT(*) + COUNT(DISTINCT caller_id)`; bounded by
      `created_at >= NOW() - $window`. Drops the events `SUM(amount)` query.
- [x] `ContractStats` DTO carries `recent_invocations`,
      `recent_unique_callers`, `stats_window`.
- [x] `fetch_contract` projects `wasm_uploaded_at_ledger` and decodes
      `contract_type_name()` server-side (paired with raw SMALLINT).
- [x] Wire field rename `deployer_account` → `deployer` (canonical).
- [x] E11 response is canonical raw-JSONB `{ contract_id, wasm_hash,
  interface_metadata }`; LEFT JOIN with `CASE` preserves the
      task-0153 stub filter; 200 + null for SAC, 404 only on miss.
- [x] E13 dropped read-time XDR fetch (canonical 13: out of scope);
      `InvocationItem` is per-appearance with `caller_account`, `amount`,
      `successful`.
- [x] E14 keeps archive overlay but enriches per-event row with
      `transaction_id`, `successful`, `amount` (canonical 14 fields).
- [x] Adopt `crate::common::*` helpers (task 0043): drops
      `contracts/cursor.rs`, hand-rolled `err()` / `is_valid_strkey` /
      `ListParams` / `resolve_list_params` / `ListParamsOutcome` in
      favour of `common::cursor::TsIdCursor`, `common::errors::*`,
      `common::extractors::Pagination<P>`.
- [x] Workspace clippy `-D warnings` clean; 80/80 tests pass live
      (including `contracts_detail_returns_canonical_shape_against_real_db`).

## Notes

- Wire-shape breaking change for E10/E11/E13/E14. Frontend has not yet
  shipped contract pages, so impact is internal only.
- The 0167 audit also flagged E13/E14 sort-order divergence (MEDIUM)
  but recommended **Option B** — implementation sort wins, canonical
  adapts. No impl change needed; Filip M updates `13_*.sql` / `14_*.sql`
  and the matching index lives under task 0132.
- The E11 stub filter (`metadata ? 'functions'`) is post-canonical
  innovation; Filip M to update `12_get_contracts_interface.sql`.
