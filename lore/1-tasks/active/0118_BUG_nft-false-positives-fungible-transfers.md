---
id: '0118'
title: 'BUG: NFT false positives from fungible token transfers'
type: BUG
status: active
related_adr: ['0012']
related_tasks: ['0026', '0027', '0140']
tags:
  [
    priority-high,
    effort-medium,
    layer-indexer,
    audit-F9,
    pending-adr-0012-review,
  ]
milestone: 1
links:
  - crates/xdr-parser/src/nft.rs
  - docs/audits/2026-04-10-pipeline-data-audit.md
history:
  - date: '2026-04-10'
    status: backlog
    who: stkrolikiewicz
    note: 'Spawned from pipeline audit finding F9 (HIGH severity).'
  - date: '2026-04-14'
    status: active
    who: fmazur
    note: 'Activated task for implementation.'
  - date: '2026-04-17'
    status: active
    who: stkrolikiewicz
    note: >
      Audit per task 0140 — ADR 0012 affects referenced schema/flow (see body
      for OLD patterns). This task is NOT hard-blocked by the migration (logic
      is schema-adjacent, not schema-gated). Verify target tables/flow against
      ADR 0012 before implementing.
---

> **⚠ Post-ADR 0012 re-read required (audit 2026-04-17, [task 0140](0140_DOCS_audit-lore-tasks-adr-0011-0012.md)):**
> Body below references pre-ADR-0012 patterns (flow, schema, upsert, partitioning). [ADR 0012](../../2-adrs/0012_zero-upsert-schema-full-fk-graph.md) supersedes the schema and ingestion flow but this task is not hard-blocked by the migration — verify target table/column/flow references against ADR 0012 before implementing.

---

# BUG: NFT false positives from fungible token transfers

## Summary

`looks_like_token_id()` in `nft.rs:171-174` accepts `i128` data, which is the standard
SEP-0041 fungible token transfer amount type. Every fungible token transfer (USDC, XLM
wrapping, etc.) creates a spurious record in the `nfts` table.

## Context

SEP-0041 fungible token transfers use the same topic pattern as NFT transfers:
`["transfer", Address(from), Address(to)]` with `i128` amount as data. The current filter
excludes `void`, `map`, `vec`, `error` — but not numeric types like `i128`, `i64`, `u128`.

At mainnet scale this will flood the `nfts` table with millions of false-positive records.

**Note (2026-04-13):** The code now has a doc comment (`nft.rs:162-170`) explicitly
acknowledging this limitation — some NFT contracts (e.g. jamesbachini) use `i128` for
token IDs, so a blanket numeric exclusion would cause false negatives. A test
`i128_token_id_not_excluded` (lines 262-277) asserts the current behavior. The bug is
recognized but intentionally deferred pending a proper fix via WASM spec analysis.

## Implementation

**Caution:** Some NFT contracts use `i128` as token IDs. A blanket numeric exclusion would
cause false negatives. The fix must distinguish between fungible amounts and NFT token IDs.

### Chosen approach: WASM classification + contract_type enrichment + post-backfill cleanup

**Decision (2026-04-14, fmazur):** Use WASM spec analysis to enrich existing `contract_type`
column in `soroban_contracts`. No new tables needed — temporary false positives in `nfts`
during parallel backfill are cleaned up with a post-backfill DELETE.

#### WASM classification logic

Based on `contractspecv0` function signatures (already extracted and stored in
`wasm_interface_metadata` JSONB):

```
IF  has owner_of OR token_uri       → contract_type = 'nft'
ELIF has decimals AND balance→i128  → contract_type = 'fungible'
ELSE                                → contract_type = 'other' (unchanged)
```

Key discriminators (SEP-0050 NFT vs SEP-0041 fungible):

| Function                       | NFT | Fungible |
| ------------------------------ | :-: | :------: |
| `owner_of(token_id) → Address` | yes |    no    |
| `token_uri(token_id) → String` | yes |    no    |
| `decimals() → u32`             | no  |   yes    |
| `allowance(...) → i128`        | no  |   yes    |

SACs (Stellar Asset Contracts) have no WASM — already classified as `'token'`.

**Precedence rule:** If a contract implements both NFT and fungible interfaces (dual-interface),
NFT classification wins. This is the safer choice — avoids data loss (false negatives).
Fungible transfer events from such contracts may appear as false positives in `nfts`, but
this is preferable to missing real NFTs.

#### Flow

1. On event detection, lookup `contract_type` for the emitting contract
   (with in-memory `HashMap<contract_id, Classification>` cache per worker process).
2. **Classified as NFT** (`contract_type = 'nft'`) → insert into `nfts`
3. **Classified as fungible** (`contract_type = 'fungible'` or `'token'`) → **skip**
4. **Unclassified** (`contract_type = 'other'`, no WASM metadata yet) → **insert into
   `nfts`** (temporary false positive, cleaned up post-backfill)
5. **Post-backfill cleanup:** DELETE from `nfts` where `contract_type IN ('fungible', 'token')`

#### Parallel backfill behavior

With 4+ workers on different ledger ranges, race conditions are possible:

1. Worker 3 (ledger 50000) — sees transfer from contract X, no WASM metadata yet →
   `contract_type = 'other'` → inserts into `nfts` (temporary false positive)
2. Worker 1 (ledger 10000) — processes WASM upload of contract X → sets
   `contract_type = 'fungible'`
3. At this point `soroban_contracts` says `'fungible'`, but the false record from
   step 1 remains in `nfts`
4. Subsequent transfers of contract X on workers 2, 4 — see `'fungible'` in cache
   → skip, no new false positives
5. Post-backfill cleanup DELETE removes the record from step 1

Temporary false positives are limited because:

- Each contract has one WASM upload and potentially thousands of transfers
- Only transfers processed **before** the WASM upload is processed are false positives
- The rest already hit the cache with correct classification

#### Implementation notes

1. **Cache must NOT store `'other'`:** Only cache definitive classifications (`'nft'`,
   `'fungible'`, `'token'`). If a contract is `'other'` (no WASM metadata yet), always
   re-query the DB on next encounter. Otherwise a single cache miss at the start of a
   worker's range causes ALL subsequent transfers for that contract to be false positives
   — even if another worker has since classified it.

2. **`contract_type` UPDATE path:** `update_contract_interfaces_by_wasm_hash()` in
   `soroban.rs:175-192` currently only updates `metadata` JSONB — it does NOT set
   `contract_type`. The `upsert_contract_deployments_batch()` uses COALESCE (first write
   wins), so it won't overwrite either. Classification must be a separate UPDATE or an
   extension of `update_contract_interfaces_by_wasm_hash()` that also sets `contract_type`
   based on the function signatures in the WASM metadata.

3. **Filtering belongs in `persist.rs`, not `process.rs`:** `detect_nft_events()` in
   `nft.rs` is a pure function in the `xdr-parser` crate (no DB access). Filtering by
   `contract_type` requires DB lookup, so it must happen in `persist.rs` before
   `upsert_nfts_batch()` (line 338), not in the parse phase. This preserves the clean
   parse/persist separation.

4. **Cache lives at worker level, not per-ledger:** `process_ledger()` currently has no
   persistent state across ledgers — it takes `(meta, pool, cw_client)`. The cache must
   be held by the caller (worker-level state), passed into `persist_ledger()` as a
   parameter. This requires a function signature change or a wrapper struct.

5. **Batch cache population:** At backfill scale, lazy per-contract DB queries are too
   expensive. For each ledger, collect distinct `contract_id`s from NFT event candidates,
   batch-query their `contract_type`, and populate the cache in one round-trip. Cache hits
   on subsequent ledgers avoid repeated queries.

#### Post-backfill cleanup script

```sql
BEGIN;
-- 1. Sanity check: ensure classification is complete
SELECT COUNT(*) AS unclassified
FROM soroban_contracts
WHERE contract_type = 'other'
  AND contract_id IN (SELECT DISTINCT contract_id FROM nfts);
-- If >0: stop, investigate unclassified contracts first

-- 2. Remove false positives
DELETE FROM nfts
WHERE contract_id IN (
    SELECT contract_id FROM soroban_contracts
    WHERE contract_type IN ('fungible', 'token')
);
COMMIT;
-- 3. Reclaim space
VACUUM ANALYZE nfts;
```

#### Why this over a staging table

- **No schema changes** — no new `nft_candidates` table, no migration
- **Less code** — no resolve logic, no retention policy, no staging table cleanup
- **Same classification mechanism** — both approaches use WASM spec analysis identically
- **Acceptable tradeoff** — temporary false positives in `nfts` during backfill only,
  automatically limited by worker ordering, cleaned up in one DELETE

#### Rejected alternatives

- **Staging table (`nft_candidates`):** Insert unknowns into separate table, resolve
  after backfill. Rejected: adds schema complexity and resolve code for marginal benefit
  over post-backfill DELETE.
- **Heuristic refinement:** Exclude `i128`/`u128` + whitelist. Rejected: fragile,
  requires manual contract classification.
- **Simple numeric exclusion:** Exclude all numeric ScVal types. Rejected: false
  negatives for NFT contracts using numeric token IDs (e.g. jamesbachini uses `i128`).

## Acceptance Criteria

### Classification

- [ ] WASM classification function: inspect `wasm_interface_metadata` JSONB for NFT-specific
      functions (`owner_of`, `token_uri`) vs SEP-0041 fungible-only (`decimals`, `allowance`)
- [ ] Enrich `contract_type` in `soroban_contracts`: set `'nft'` or `'fungible'` based on
      WASM classification (during WASM metadata persist step in `persist.rs`)
- [ ] In-memory cache (`HashMap<contract_id, Classification>`) per worker to avoid repeated
      DB lookups — only cache definitive classifications (`'nft'`, `'fungible'`, `'token'`),
      never cache `'other'` (re-query DB on next encounter)
- [ ] Batch cache population: collect distinct contract_ids from NFT candidates per ledger,
      batch-query `contract_type`, populate cache in one round-trip
- [ ] Cache lives at worker level (not per-ledger) — requires passing state into
      `persist_ledger()` or wrapping in a struct

### NFT detection filtering

- [ ] Classified NFT (`contract_type = 'nft'`) → insert into `nfts`
- [ ] Classified fungible (`contract_type = 'fungible'` or `'token'`) → skip (no insert)
- [ ] Unclassified (`contract_type = 'other'`, no WASM metadata yet) → insert into `nfts`
      (temporary false positive, accepted during backfill)

### Post-backfill cleanup

- [ ] Manual SQL cleanup script: DELETE from `nfts` where `contract_type IN ('fungible', 'token')`
      in a single transaction, with sanity check for unclassified contracts first
- [ ] `VACUUM ANALYZE nfts` after cleanup

### Live indexing

- [ ] Verify that 2-ledger pattern (WASM upload → deploy) guarantees metadata availability
      before first transfer event; document any edge cases

### Tests

- [ ] Remove or update test `i128_token_id_not_excluded` — it currently asserts the broken
      behavior
- [ ] Existing NFT detection tests still pass (for string/bytes token_id contracts)
- [ ] New test: SEP-0041 fungible transfer (i128) + fungible-classified contract → no NFT record
- [ ] New test: NFT contract (WASM-classified) with i128 token_id → detected correctly
- [ ] New test: unknown contract (no WASM metadata) with i128 data → still inserted into `nfts`
