---
id: '0172'
title: 'BUG: contracts E10 stats — bounded window + correct metric + missing E10 fields per task 0167 audit'
type: BUG
status: active
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
      Spawned from task 0167 audit on PR #126 (task 0050 contracts module).
      Canonical SQL `11_get_contracts_by_id.sql` landed 8 minutes before
      0050 merged so the divergences are a historical artifact, not a
      deliberate departure. Critical issue: `fetch_contract_stats` does
      a full-history scan over both partitioned appearance tables on every
      cache miss, with `SUM(amount)` (per-row appearance counter) instead
      of canonical's `COUNT(*) + COUNT(DISTINCT caller_id)` over a bounded
      time window. Production blocker at mainnet scale.
  - date: '2026-04-28'
    status: active
    who: FilipDz
    note: >
      Activated immediately after 0049 (PR #134) merged to develop.
      Picking up the audit fix as a follow-up to 0050.
---

# BUG: contracts E10 stats — bounded window + correct metric + missing E10 fields

## Summary

Align `crates/api/src/contracts/` with the canonical SQL deliverable from
task 0167 (`docs/architecture/database-schema/endpoint-queries/11_get_contracts_by_id.sql`).
The audit on PR #126 flagged one HIGH and several LOW severity divergences;
this task fixes them in one PR.

## Status: Backlog

**Current state:** Audit findings recorded; no code changes yet.

## Audit findings (from PR #126 / task 0167 audit)

### HIGH — `fetch_contract_stats` unbounded scan + wrong metric

Current code does:

```sql
SELECT COALESCE(SUM(amount)::BIGINT, 0)
FROM soroban_invocations_appearances WHERE contract_id = $1
-- + same SUM(amount) over soroban_events_appearances
```

Two issues bundled here:

1. **No time bound** — every cache miss triggers a full-history partition
   scan against both `soroban_invocations_appearances` and
   `soroban_events_appearances`. The 45 s `ContractMetadataCache` warms
   popular contracts but long-tail and freshly indexed contracts always
   cold-miss to a full scan. Same scaling profile as the `count(*) FROM
accounts` issue flagged on PR #125.
2. **Wrong metric semantics** — `amount` in the appearance tables is a
   per-row counter on the deduplicated `UNIQUE NULLS NOT DISTINCT` tuple
   (ADR 0034). `SUM(amount)` and `COUNT(*)` answer different questions;
   the frontend's "Invocations" cell is currently driven by a number
   whose meaning depends on how the indexer increments `amount`, not by
   the row count canonical specifies.

Canonical (`11_*.sql` Statement B):

```sql
SELECT
    COUNT(*)                          AS recent_invocations,
    COUNT(DISTINCT sia.caller_id)     AS recent_unique_callers,
    $2::interval                      AS stats_window
FROM soroban_invocations_appearances sia
WHERE sia.contract_id = $1
  AND sia.created_at >= NOW() - $2::interval;
```

Note: canonical does NOT specify event stats — drop the events
`SUM(amount)` query. If the frontend later needs event counts, expose
as a separate bounded query in a follow-up.

### LOW — `fetch_contract` missing fields

Canonical projection includes `wasm_uploaded_at_ledger` and a decoded
`contract_type_name(sc.contract_type)` label paired with the raw SMALLINT.
PR #126 omitted `wasm_uploaded_at_ledger` and decoded `contract_type` in
Rust instead of via the SQL helper. Aligning to canonical's pair pattern
matches what tasks 0049 / 0050 already do for the `assets` module.

### LOW — wire field naming

Canonical aliases: `deployer` (not `deployer_account`), `wasm_hash_hex`
(not `wasm_hash`). Pick one set and update both implementation and
canonical. Recommendation: implementation wins (canonical aliases are
internal SQL doc; DTO is the wire contract). Filip M can update
`11_*.sql` aliases to match the DTO names. **Or** rename the DTO to
match canonical — decide as part of this task.

## Implementation plan

### Step 1 — `fetch_contract` projection

- Add `sc.wasm_uploaded_at_ledger`
- Add `contract_type_name(sc.contract_type) AS contract_type_name`
- Update `ContractRow` struct accordingly

### Step 2 — `fetch_contract_stats` rewrite

- Replace signature: `fn fetch_contract_stats(pool, contract_id, stats_window: &str) -> (i64, i64, String)`
- New SQL: `COUNT(*), COUNT(DISTINCT caller_id), $2::text` with
  `WHERE contract_id = $1 AND created_at >= NOW() - $2::interval`
- Bind the window string twice ($2 used as both interval and label).
- Drop the events SUM(amount) query entirely.
- Default window: `"7 days"` (audit recommendation; canonical example).
  Hardcode as a `const` in the handler — no query-param plumbing yet.

### Step 3 — DTO update (breaking)

```rust
pub struct ContractStats {
    pub recent_invocations: i64,
    pub recent_unique_callers: i64,
    pub stats_window: String,        // e.g. "7 days"
}

pub struct ContractDetailResponse {
    pub contract_id: String,
    pub wasm_hash: Option<String>,                // hex
    pub wasm_uploaded_at_ledger: Option<i64>,     // NEW
    pub deployer: Option<String>,                  // RENAMED from deployer_account
    pub deployed_at_ledger: Option<i64>,
    pub contract_type_name: Option<String>,        // decoded via SQL helper
    pub contract_type: Option<i16>,                // raw SMALLINT
    pub is_sac: bool,
    pub metadata: Option<serde_json::Value>,
    pub stats: ContractStats,
}
```

### Step 4 — handler wiring

`get_contract` threads the `STATS_WINDOW` const through to
`fetch_contract_stats`. Cache shape changes (cached value now carries
`recent_*` fields and `stats_window` label) — no migration needed since
the cache is per-Lambda warm only.

### Step 5 — live integration test

Add a DB-gated `contracts_detail_returns_canonical_shape_against_real_db`
test that asserts every canonical field is present and the stats trio
shape (numbers + window label string).

## Acceptance criteria

- [ ] `fetch_contract_stats` queries `soroban_invocations_appearances`
      ONLY (no events table); uses `COUNT(*) + COUNT(DISTINCT caller_id)`;
      bounded by `created_at >= NOW() - $window`.
- [ ] `Stats` DTO carries `recent_invocations`, `recent_unique_callers`,
      `stats_window` (echoed back as a label).
- [ ] `fetch_contract` projects `wasm_uploaded_at_ledger` and decodes
      `contract_type_name()` server-side; both surfaced on the response
      alongside the raw SMALLINT.
- [ ] DTO field naming reconciled with canonical (decision documented in
      history).
- [ ] Workspace clippy clean (`-D warnings`); api crate tests pass.
- [ ] At least one live integration test locks the canonical-aligned
      response shape.

## Notes

- This is a wire-shape breaking change for E10. Frontend has not yet
  shipped a contract detail page (per status of `web/`), so impact is
  internal only. Coordinate with Karol/UI before merge regardless.
- The 0167 audit also flagged E13/E14 sort-order divergence as MEDIUM,
  but recommended **Option B** (PR #126's sort wins, canonical SQL
  adapts, supporting index moves to task 0132). No code change needed
  here; that's Filip M's update to canonical.
- The E11 stub filter (`metadata ? 'functions'`) is post-canonical
  innovation; canonical SQL `12_get_contracts_interface.sql` should be
  updated by Filip M to incorporate. Out of scope for this task.
