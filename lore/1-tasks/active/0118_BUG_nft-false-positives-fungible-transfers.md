---
id: '0118'
title: 'BUG: NFT false positives from fungible token transfers'
type: BUG
status: active
related_adr: []
related_tasks: ['0026', '0027']
tags: [priority-high, effort-medium, layer-indexer, audit-F9]
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

### Chosen approach: WASM classification + staging table

**Decision (2026-04-14, fmazur):** Use WASM spec analysis with `nft_candidates` staging
table to guarantee zero data loss during parallel backfill.

#### Flow

1. On event detection, lookup `wasm_interface_metadata` for the emitting contract
   (with in-memory `HashMap<contract_id, Classification>` cache per worker process).
2. **Classified as NFT** (has `token_uri`, `owner_of`) → insert directly into `nfts`
3. **Classified as fungible** (SEP-0041 only, no NFT functions) → **skip**
4. **No WASM metadata available** (race condition during parallel backfill, or contract
   not yet indexed) → insert into **`nft_candidates`** staging table
5. **Post-backfill resolve:** query `nft_candidates` against `wasm_interface_metadata`,
   move confirmed NFTs to `nfts`, discard the rest

#### Why staging over direct cleanup

- `nfts` table stays clean — no millions of false positives inflating indexes
- No expensive mass DELETE + VACUUM after backfill
- `nft_candidates` is small (only unknown contracts) and disposable
- Safe for parallel backfill with 4+ workers on different ledger ranges — race
  conditions on WASM metadata availability are handled gracefully with zero data loss

#### Rejected alternatives

- **Direct cleanup (option 3):** Record everything to `nfts`, DELETE after backfill.
  Rejected: table bloat, expensive cleanup, dead tuples.
- **Heuristic refinement:** Exclude `i128`/`u128` + whitelist. Rejected: fragile,
  requires manual contract classification.
- **Simple numeric exclusion:** Exclude all numeric ScVal types. Rejected: false
  negatives for NFT contracts using numeric token IDs.

## Acceptance Criteria

- [ ] WASM classification: lookup `wasm_interface_metadata` for NFT-specific functions
      (`token_uri`, `owner_of`) vs SEP-0041 fungible-only functions
- [ ] In-memory cache (`HashMap<contract_id, Classification>`) to avoid repeated DB lookups
- [ ] Classified NFT → insert directly into `nfts`
- [ ] Classified fungible → skip (no insert)
- [ ] Unknown contract (no WASM metadata) → insert into `nft_candidates` staging table
- [ ] Migration: create `nft_candidates` staging table (mirror `nfts` columns + `staged_at`
      timestamp) — verify actual `nfts` schema from migrations before writing
- [ ] Post-backfill resolve: manual SQL script (not automated) to move confirmed NFTs
      from `nft_candidates` to `nfts`, discard fungible false positives. Must run in a
      single transaction (INSERT + DELETE) to prevent data loss on failure
- [ ] Retention policy for `nft_candidates`: contracts that remain unresolvable (no WASM
      metadata after full backfill) should be flagged or purged — table must not grow unbounded
- [ ] Live indexing path: verify that 2-ledger pattern (WASM upload → deploy) guarantees
      metadata availability before first transfer event; document any edge cases
- [ ] Remove or update test `i128_token_id_not_excluded` — it currently asserts the broken
      behavior
- [ ] Existing NFT detection tests still pass (for string/bytes token_id contracts)
- [ ] New test: SEP-0041 fungible transfer (i128) + fungible-classified contract → no NFT record
- [ ] New test: NFT contract (WASM-classified) with i128 token_id → detected correctly
- [ ] New test: unknown contract (no WASM metadata) with i128 data → goes to `nft_candidates`
- [ ] New test: resolve step moves NFT candidates to `nfts` after WASM metadata available
