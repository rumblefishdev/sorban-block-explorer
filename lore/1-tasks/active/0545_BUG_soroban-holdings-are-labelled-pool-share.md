---
id: '0496'
title: 'BUG: every Soroban holding is labelled `pool_share` — two asset-type enums collided'
type: BUG
status: active
related_adr: ['0051']
related_tasks: ['0463', '0331', '0339']
tags: [backend, api, accounts, data-correctness, priority-medium, effort-small]
links: ['https://github.com/rumblefishdev/soroban-block-explorer/issues/377']
history:
  - date: '2026-08-27'
    status: active
    who: karolkow
    note: >
      Pulled forward during the 0374 schema review: an adversarial pass found
      the mirror defect (pool-leg DTO documents the family legend over XDR
      values), which makes this one bug with two public faces. Fixed together
      with renaming the project enum so the collision cannot recur silently.
  - date: '2026-08-17'
    status: backlog
    who: karolkow
    note: >
      Found while auditing why LP positions live outside `balances` (0463
      planning). Two enums share the number 3 with different meanings, and the
      API renders the wrong one. Measured on production before filing.
---

# BUG: Soroban holdings render as `pool_share`

## Summary

The account detail response labels **every Soroban (type-3) holding** as
`pool_share`. Measured on production: **42,975 rows across 38,324 holders**
carry `asset_type = 3` with a non-zero amount, and every one of them is
mislabelled.

## Root cause — two enums, one number

```
TokenAssetType  (stored in `assets.asset_type`)   Native=0, ClassicCredit=1, Soroban=3
asset_type_name (rendered by the API)             0 native, 1 credit_alphanum4,
                                                  2 credit_alphanum12, 3 pool_share
```

`crates/domain/src/enums/token_asset_type.rs:24-30` is the project enum;
`crates/api/src/accounts/queries.rs:102-110` is the renderer, which uses the
**XDR** meaning of 3 (`AssetType::PoolShare`, confirmed in `stellar-xdr`
26.0.1). A Soroban holding therefore reads as a liquidity-pool share.

The collision was created, not inherited: ADR 0051 / task 0339 retired type 2
(SAC) and Soroban took 3 — the number XDR already uses for `PoolShare`. The
renderer was never revisited.

Nothing caught it because our real pool shares are not in `balances` at all —
they live in `lp_positions` — so the `pool_share` label had no legitimate
occurrence to be compared against.

## Second defect in the same function

The doc comment calls these "Horizon-style" labels, but Horizon does not emit
`pool_share`. Verified against the live API and the Go SDK
(`protocols/horizon/main.go`): the value is **`liquidity_pool_shares`**. So the
string is wrong even for the case it was written for.

## Fix

- Render from the **project** enum, not the XDR one: type 3 must produce a
  Soroban label, not `pool_share`.
- Decide the exact string deliberately — it is a wire-visible contract
  (`libs/api-types/src/generated/types.gen.ts:20`). Horizon has no Soroban
  equivalent, so this is our own vocabulary; pick it once and use it
  everywhere.
- If a pool-share label is ever needed, it is `liquidity_pool_shares`.
- Check the other renderers for the same collision — `asset_type_name` is
  duplicated in the search path (`search/queries.rs`) and possibly elsewhere.
- Regression test pinning type 3 → the Soroban label.

## Acceptance criteria

- [ ] A Soroban holding no longer reads as `pool_share` on the account page
- [ ] Every copy of the type → label mapping agrees, with one shared source
- [ ] The chosen string is recorded, with the reason it is not Horizon's
- [ ] **Docs updated** — frontend data contract, since the value is on the wire
- [ ] **API types regenerated** if the field's documented values change
      (`npx nx run @rumblefish/api-types:generate`)

## Implementation — 2026-08-27

### What shipped

1. **The renderer speaks its own domain.** `accounts/queries.rs`
   `asset_type_name()` now derives the label from `AssetFamily::as_str()`
   instead of hand-copying the XDR legend: 3 → `soroban`, 1 →
   `classic_credit`, 2 → `None` (retired). This also HEALS an API
   inconsistency nobody had filed: `/assets` already spoke the family
   vocabulary, so one token read "Soroban" on the assets list and
   "pool_share" on the account page. The frontend's own `assetType.ts` was
   already keyed to the family words and branches only on `native` — no FE
   change needed.
2. **The mirror defect** (found by the schema review's adversarial pass):
   `liquidity_pools/dto.rs` documented the FAMILY legend on the pool-leg
   `asset_type`, which carries XDR values — declaring 2 "retired" while
   54 456 production legs carry 2 = `credit_alphanum12`. Legend corrected to
   the XDR vocabulary on both fields.
3. **`TokenAssetType` renamed to `AssetFamily`** across the workspace, so the
   two enums no longer share the `…AssetType` suffix. `AssetKind` was
   rejected: `asset_kind` is the pinned interop key of the external
   `prices.*` views. The SQL column keeps its name — it is the first
   sort-key column of `assets` and ClickHouse refuses to rename key columns
   (verified; error 524) — so the Rust-side name is the disambiguator.
4. **The pinning test rewritten.** `asset_type_name_matches_pg_function`
   asserted the wrong mapping (3 = `pool_share`) and kept the bug green;
   it now asserts the family vocabulary and documents why.
5. **Wire label for the future classic pool-share variant decided and
   recorded in 0499**: `liquidity_pool_shares` (the ecosystem word, verified
   against the Go SDK) — not `pool_share`, which no consumer emits.
6. api-types regenerated; diff is descriptions only.

### The rule this leaves behind

A renderer may only use the vocabulary of the enum its value came from.
Two enums may map the same integer; the LABEL functions must never cross.

### Verified

747 workspace tests green (258 domain / 15+api / 100 db-clickhouse / 19
indexer / 355 xdr-parser), clippy clean. Production re-verification of the
account response happens at deploy per the release flow.
