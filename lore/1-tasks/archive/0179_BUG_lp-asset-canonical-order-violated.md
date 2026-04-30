---
id: '0179'
title: 'BUG: 40 liquidity_pools rows have asset_a > asset_b (Stellar canonical order violated)'
type: BUG
status: completed
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
  - date: '2026-04-30'
    status: completed
    who: stkrolikiewicz
    note: >
      Closed via PR #147 (I3 rewritten to drop surrogate-ID compare,
      `(type, code)` levels remain enforced) and PR #151
      (`archive-diff --table liquidity_pools` adds `pool_id ==
      SHA-256(LiquidityPoolParameters XDR)` verifier per CAP-0038,
      closing the deferred issuer-level acceptance criterion).
      Validated on a clean-slate 1k-spot re-backfill: I3 reports 0
      violations and 500 / 500 sampled pools pass the protocol-hash
      check. The 40 rows flagged on the pre-fix 30k smoke were false
      positives from the surrogate-ID compare, not data corruption.
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

## Investigation (2026-04-29)

The flagged 40 rows are NOT a parser/data bug. The invariant test
itself is broken: it compares fields that cannot represent Stellar's
canonical order in this schema.

### Why the original I3 produces false positives

`audit-harness/sql/15_liquidity_pools.sql:23-24` (pre-fix) compared:

```sql
asset_a_type = asset_b_type
  AND asset_a_code = asset_b_code
  AND asset_a_issuer_id > asset_b_issuer_id
```

`asset_a_issuer_id` is a surrogate `BIGINT` FK to `accounts.id` (per
ADR 0026 + 0030), assigned in **insertion order** — i.e. whichever
account was upserted first by the indexer. Stellar's canonical order
at the issuer level uses the issuer's **ed25519 raw bytes**. The two
have zero correlation: same-(type, code) different-issuer pools where
the two issuers happened to be inserted in reverse-of-canonical order
get flagged. The 40 violations on the 30k smoke are exactly such pools,
not flipped pairs.

The natural key `accounts.account_id` is a base32-encoded G-strkey,
but ASCII lex compare on base32 strings is also non-monotonic for raw
byte order (the alphabet maps `A-Z`=0-25, `2-7`=26-31, but ASCII sorts
digits BEFORE letters), so `account_id > account_id` is not a valid
substitute either.

### Why the parser is correct

[`extract_liquidity_pools`](../../../crates/xdr-parser/src/state.rs#L406)
reads `cp.params.asset_a` / `cp.params.asset_b` from the
`LiquidityPoolEntry.body.LiquidityPoolConstantProduct.params` field of
the XDR ledger entry, which is the canonicalized post-deposit state
the protocol writes. No reordering happens in our extractor (lines
[440-447](../../../crates/xdr-parser/src/state.rs#L440)). The `pool_id`
itself is read from the same entry via
[`liquidity_pool_data`](../../../crates/xdr-parser/src/ledger_entry_changes.rs#L426)
and matches `SHA-256(LiquidityPoolParameters XDR)` per CAP-0038 by
construction.

### What this PR changes (audit-harness only)

Rewrite I3 to drop the surrogate-ID comparison. The (type, code) levels
of canonical order ARE expressible against our schema (`asset_*_type
SMALLINT`, `asset_*_code TEXT`) and remain enforced. The issuer level
defers to Phase 2c `archive-diff --table liquidity_pools`, which
re-parses the XDR and verifies `pool_id` matches the protocol-derived
hash directly — that catches any actual flipped pair regardless of
which end of canonical order the issuer falls on. A SQL-only
re-derivation would require base32-decoding the strkey to ed25519 raw
bytes; not worth the surface area for an invariant already covered.

The full reasoning is preserved in the rewritten I3 SQL header
comment so future readers don't re-derive it.

## Acceptance Criteria

- [x] Identify which extraction path emits flipped pairs — none. The
      parser reads canonical pairs from the on-chain `LedgerEntry`.
- [x] Either (a) read assets from the canonical on-chain LedgerEntry
      (post-deposit state), or (b) sort the pair before persisting —
      (a) is already the case; no change needed.
- [x] Add a Phase 1 invariant: `pool_id` matches `SHA-256(canonical
asset_a, asset_b, fee_bp)` — deferred to Phase 2c
      `archive-diff --table liquidity_pools`. SQL-only check would
      require base32 decoding of issuer strkeys; reasoning recorded in
      the rewritten I3 SQL header so the deferral is discoverable.
- [x] Reindex affected pools: 40 rows + their snapshots + lp_positions
      need re-extraction. — N/A. The 40 rows ARE canonical; the bug
      was in the test, not the data. Re-backfill on develop is
      independently planned for the 0178 + 0181 acceptance gate and
      will produce a fresh dataset that passes the corrected I3 with
      0 violations.

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
