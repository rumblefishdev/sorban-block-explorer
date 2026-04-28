---
id: '0175'
title: 'Continuous data correctness audit harness — SQL invariants + DB↔Horizon + DB↔RPC + DB↔archive XDR diff'
type: FEATURE
status: active
related_adr: ['0008', '0027', '0029', '0037']
related_tasks: ['0167', '0168', '0169', '0172', '0173']
tags: [priority-high, layer-db, layer-backend, audit, backfill, correctness]
milestone: 1
links:
  - lore/3-wiki/backfill-execution-plan.md
  - lore/1-tasks/archive/0167_FEATURE_endpoint-sql-query-reference-set.md
history:
  - date: '2026-04-28'
    status: active
    who: stkrolikiewicz
    note: >
      Spawned + activated. Filip's prior audits (0167, 0172, 0173) were
      manual deep-dives spawning narrow bug fixes. This task is the
      automated complement — bulk + continuous coverage that manual
      approach can't scale to. Phase 1 is internal SQL invariants
      (Filip never touched this surface); Phases 2a/2b/2c are bulk
      DB↔Horizon / DB↔Soroban RPC / DB↔archive XDR diffs at N=1000+
      vs Filip's N=6 by hand. Findings funnel into the same bug-task
      pipeline.
---

# Continuous data correctness audit harness

## Summary

Five-phase audit harness covering all 17 tables in the schema. Phase 1
ships first as pure SQL invariants (no external dependency); Phases
2a/2b/2c add automated diff against Horizon API, Soroban RPC, and raw
archive XDR; Phase 3 is aggregate sanity for continuous monitoring.
Designed to complement, not replace, Filip's PR-driven manual audits.

## Context

Three prior audit waves found 5 bugs (0168/0169/0170/0172/0173) at
N=6 manually-picked transactions per audit. The methodology works for
deep, narrow bugs but doesn't scale: 17 tables × ~30 fields per row
× 17M ledgers = a search space the human eye cannot cover.

This harness automates the wide-but-shallow coverage — every row, every
field, against every reachable external source — and surfaces
divergences as bug tasks that Filip's narrow-deep approach can then
investigate.

**Scope vs Filip's manual audits.** Complement, not replacement:

| Approach       | Sample                    | Method                     | Catches                                                                |
| -------------- | ------------------------- | -------------------------- | ---------------------------------------------------------------------- |
| Filip (manual) | 6 hand-picked             | Eyeball + write-up         | Subtle protocol/parsing bugs (V4 meta, fee_account, application_order) |
| This harness   | 1000+ random + edge cases | Automated field-level diff | Systematic divergences (entire class wrong)                            |

## Phases

### Phase 1 — SQL invariants (no external dependency)

Pure SQL against local DB. Catches **internal consistency bugs** without
any external API. Filip's audits never touched this surface.

Coverage matrix (per table):

| Table                             | Invariant                                                                      |
| --------------------------------- | ------------------------------------------------------------------------------ |
| `ledgers`                         | sequence monotonic + contiguous, hash UNIQUE                                   |
| `transactions`                    | hash UNIQUE across partitions, op_count = COUNT(operations_appearances) per tx |
| `transaction_hash_index`          | every hash → matching transactions FK                                          |
| `operations_appearances`          | FK to transactions valid, appearance_count semantics (ADR 0037 §7)             |
| `transaction_participants`        | (account_id, tx_id) UNIQUE, FK valid                                           |
| `soroban_contracts`               | C-prefix StrKey shape, deployer_id FK, type SMALLINT range                     |
| `wasm_interface_metadata`         | wasm_hash UNIQUE, every soroban_contracts.wasm_hash exists                     |
| `soroban_events_appearances`      | FK to transactions valid                                                       |
| `soroban_invocations_appearances` | FK to transactions valid                                                       |
| `assets`                          | ck*assets_identity per ADR 0038, uidx_assets*\* enforced                       |
| `accounts`                        | G-prefix StrKey shape, first_seen ≤ last_seen                                  |
| `account_balances_current`        | balance ≥ 0, partial uidx native vs credit                                     |
| `nfts`                            | (contract_id, token_id) UNIQUE                                                 |
| `nft_ownership`                   | last row per nft → matches nfts.current_owner_id                               |
| `liquidity_pools`                 | pool_id matches expected hash, asset_a < asset_b                               |
| `liquidity_pool_snapshots`        | total_shares ≥ 0                                                               |
| `lp_positions`                    | shares > 0 partial, sum = snapshot.total_shares within stale tolerance         |
| **All partitioned**               | every row's `created_at` routes to its actual containing partition             |

Deliverable: bash script `crates/audit-harness/sql-invariants.sh` (or
similar) that runs each invariant as a SELECT and reports rows that
violate. Zero rows = green. Output structured for human review + CI
integration.

### Phase 2a — DB vs Horizon API (classic tables)

Rust binary `crates/audit-harness/bin/horizon-diff.rs`. For N random
rows per table, fetch our row + Horizon equivalent + diff field-level.

Tables: `ledgers`, `transactions`, `accounts`, `account_balances_current`,
`assets`, `liquidity_pools`.

### Phase 2b — DB vs Soroban RPC (Soroban tables)

Same harness, different transport. Tables: `soroban_contracts`,
`wasm_interface_metadata`, `soroban_events_appearances`,
`soroban_invocations_appearances`.

### Phase 2c — DB vs archive XDR re-parse (ground truth)

Independent re-parse path. Pick N random ledgers, fetch from public
archive, run an independent extractor (not `crates/xdr-parser`),
diff every extracted field against DB. **Catches parser bugs** —
this is the strongest correctness check available. Most expensive,
runs once pre-T0.

### Phase 3 — Aggregate sanity (continuous monitoring)

Daily counts vs known network growth curves. Asset registry sweep
vs SDF Anchored Assets. Cron-friendly script.

## Acceptance Criteria (per phase)

### Phase 1 (this PR)

- [ ] `crates/audit-harness/sql-invariants.sh` runs against local DB
- [ ] Each of the 17 tables has at least one invariant check
- [ ] All partitioned tables have a partition-routing check
- [ ] Output is operator-readable (table name → invariant → violation count + sample)
- [ ] Run against a populated 30k smoke backfill produces a baseline report
- [ ] Findings (if any) recorded as bug tasks via the standard spawn flow

### Phases 2a/2b/2c/3

Deferred — separate PRs after Phase 1 ships and we have post-T0 data
to validate against.

## Out of scope

- Replacing Filip's manual audits — this harness funnels findings into
  the same bug-task pipeline; Filip continues PR-driven deep dives
- Performance benchmarking — separate concern (see task 0149)
- Real-time alerting — Phase 3 emits batch reports, not pagers

## Notes

The 30k smoke backfill (ledgers 62016000–62046000, ~5 days mainnet)
runs against the freshly-restored Docker config (external SSD,
fsync=off, 2GB shared_buffers). Phase 1 SQL invariants are written
against the schema in [ADR 0037](../../2-adrs/0037_current-schema-snapshot.md)
and validated against that smoke dataset before the full T0 run.
