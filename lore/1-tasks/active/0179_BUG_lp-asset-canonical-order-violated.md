---
id: '0179'
title: 'BUG: 40 liquidity_pools rows have asset_a > asset_b (Stellar canonical order violated)'
type: BUG
status: active
related_adr: ['0026', '0030', '0037']
related_tasks: ['0126', '0162', '0175']
tags:
  [
    priority-medium,
    layer-audit-harness,
    audit-driven,
    liquidity-pools,
    false-positive,
  ]
links:
  - crates/audit-harness/sql/15_liquidity_pools.sql
  - crates/xdr-parser/src/state.rs
  - lore/2-adrs/0037_current-schema-snapshot.md
history:
  - date: '2026-04-28'
    status: backlog
    who: stkrolikiewicz
    note: >
      Surfaced by task 0175 Phase 1 SQL invariants on full 30k smoke
      backfill (ledgers 62016000–62046000). 40 rows in
      `liquidity_pools` have asset_a > asset_b under the canonical
      Stellar comparison (type, then code, then issuer). Stellar
      protocol guarantees pool asset pairs are canonicalised on
      `LiquidityPoolDeposit`, so violations indicate the parser is
      either reading the wrong fields or persisting before
      canonicalisation.
  - date: '2026-04-29'
    status: active
    who: stkrolikiewicz
    note: >
      Re-classified: NOT a parser/data bug — broken invariant test.
      I3 in `audit-harness/sql/15_liquidity_pools.sql:23-24` compares
      `asset_a_issuer_id > asset_b_issuer_id` where issuer_id is the
      surrogate BIGINT FK to `accounts` (ADR 0026/0030),
      insertion-order assigned. Stellar canonical order uses ed25519
      raw bytes of the issuer — uncorrelated with our surrogate IDs.
      Same-code-different-issuer pools where surrogates land
      reverse-of-canonical produce false positives (40/N rows).
      Parser at `state.rs:440-447` reads `cp.params.asset_a/b` from
      XDR LedgerEntry which Stellar canonicalizes at deposit time,
      so the persisted pair IS canonical. Fix: replace I3
      surrogate-ID compare with `pool_id == SHA-256(canonical pair,
      fee_bps)` protocol-derived hash check (also covers the missing
      acceptance criterion the original task flagged).
---

# `liquidity_pools` asset pair order violations

## Summary

40 rows in `liquidity_pools` violate the canonical asset ordering rule
`(asset_a_type, asset_a_code, asset_a_issuer_id) <
(asset_b_type, asset_b_code, asset_b_issuer_id)`. Stellar's protocol
guarantees this ordering on every pool — `pool_id` is in fact derived
from the pair via SHA-256 in canonical order, so any row with the
order flipped also has a `pool_id` that doesn't match what the
network would compute.

Phase 1 invariant `liquidity_pools.I3` flagged the 40 rows; sample
`pool_id` (hex):

```
0cd81ba8d81b08d0fc0141846925405d8757441fe69841570da01b8e7a08638b
1247b18e66135d6c995acd82e91fee5ebb399941ed259f46b7f82f92edd75a7f
1d061373985b2112d3cf20767c81fb7be6178eb8dec9f6f99cc736fbe5323650
202452010cd045a8449873ce57019c1bf605b944cf5c8275b6bb7ca206d038dc
33375ac45ba25e41e68acb2fed9dbecc6feecf7d8fffeb8644ef2c1d73705fcd
(plus 35 more)
```

## Reproduction

```bash
DATABASE_URL=... crates/audit-harness/run-invariants.sh
# liquidity_pools I3 violations: 40
```

```sql
SELECT count(*) FROM liquidity_pools
WHERE asset_a_type > asset_b_type
   OR (asset_a_type = asset_b_type AND asset_a_code > asset_b_code)
   OR (asset_a_type = asset_b_type AND asset_a_code = asset_b_code
       AND asset_a_issuer_id > asset_b_issuer_id);
-- 40
```

## Hypothesis

The LP extractor in `crates/xdr-parser/src/state.rs` reads asset_a /
asset_b from XDR. Two failure modes are plausible:

1. **Reading `LiquidityPoolDeposit.asset_a` / `asset_b` as-is** —
   on-chain these are emitted in canonical order, so this should be
   safe. Worth verifying.
2. **Reading from a non-canonical source** — e.g. extracting from
   user-submitted operation arguments before Stellar canonicalises,
   or reading the order in which the dev wrote the deposit op
   (which gets reordered server-side). If the parser sources from
   the operation body instead of the resulting `LedgerEntry`, the
   user-side order would be persisted.

`pool_id` is a SHA-256 input that depends on the canonical order. If
the persisted asset_a/asset_b are flipped relative to what `pool_id`
encodes, the row's `pool_id` will not equal the on-chain hash —
which is also worth a Phase 1 invariant check (currently absent).

## Acceptance Criteria

- [ ] Identify which extraction path emits flipped pairs (audit every
      `liquidity_pools` insert site in `crates/xdr-parser/src/state.rs`
      and `crates/indexer/src/handler/persist/write.rs`)
- [ ] Either (a) read assets from the canonical on-chain LedgerEntry
      (post-deposit state), or (b) sort the pair before persisting
- [ ] Add a Phase 1 invariant: `pool_id` matches `SHA-256(canonical
  asset_a, asset_b, fee_bp)` — verify against
      [task 0175 sql/15_liquidity_pools.sql](../../../crates/audit-harness/sql/15_liquidity_pools.sql)
      I3 sibling. Catches the flipped state directly via the
      protocol-derived hash, not just the comparison rule.
- [ ] Reindex affected pools: 40 rows + their snapshots + lp_positions
      need re-extraction. Document in PR commit body.

## Notes

- **Read-side impact:** any query joining on `(asset_a_code,
asset_a_issuer_id)` to identify pools containing a specific asset
  will miss 40 pools (~variable share of mainnet pools depending on
  rollout). E18 (`/liquidity-pools` list) and E20 may surface ghost
  pools or omit real ones.
- **Cross-check via Horizon:** sampling the 40 pool IDs against
  Horizon `/liquidity_pools/:id` should reveal whether Horizon
  agrees with the asset pair we have or with the flipped order. The
  Phase 2a `liquidity-pools` table diff in audit-harness already
  exists and emits `asset_a` / `asset_b` mismatches when present —
  it returned 0 mismatches on a 50-row sample, so the bug is
  concentrated in <80% of pools and easily missed by random
  sampling. Worth a targeted re-run on these specific pool_ids.
- **Companion to LP work:** task 0126 (LP participants API) and
  task 0162 (pool_share trustlines) consume `liquidity_pools`
  rows under the assumption of canonical order. Both pass on the
  current data because they don't compare asset_a vs asset_b
  directly, but a future endpoint that does (e.g. asset-filtered
  list) would mis-route reads on the affected 40 pools.
