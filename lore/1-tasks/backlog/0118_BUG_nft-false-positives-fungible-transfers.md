---
id: '0118'
title: 'BUG: NFT false positives from fungible token transfers'
type: BUG
status: backlog
related_adr: []
related_tasks: ['0026', '0027']
tags: [priority-high, effort-small, layer-indexer, audit-F9]
milestone: 1
links:
  - crates/xdr-parser/src/nft.rs
  - docs/audits/2026-04-10-pipeline-data-audit.md
history:
  - date: '2026-04-10'
    status: backlog
    who: stkrolikiewicz
    note: 'Spawned from pipeline audit finding F9 (HIGH severity).'
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

Approaches (in order of reliability):

1. **WASM spec analysis** (best): Use `wasm_interface_metadata` to check if the emitting
   contract implements NFT-specific functions (e.g., `token_uri`, `owner_of`) vs SEP-0041
   fungible functions only. Only insert into `nfts` if confirmed NFT contract.
2. **Heuristic refinement** (fallback): Exclude `i128`/`u128` by default but whitelist
   contracts known to use numeric token IDs (requires a classification pass).
3. **Simple numeric exclusion** (quick fix): Exclude all numeric ScVal types. Accepts some
   false negatives for NFT contracts using numeric IDs.

Add regression tests with i128 data simulating a standard SEP-0041 transfer.

## Acceptance Criteria

- [ ] NFT detection uses WASM spec analysis (Option 1) to classify contracts before inserting
      into `nfts` — query `wasm_interface_metadata` for NFT-specific functions (`token_uri`,
      `owner_of`, etc.) vs SEP-0041 fungible-only functions
- [ ] Fallback for contracts without WASM metadata: exclude numeric ScVal types (i128, u128,
      i64, u64) as a conservative default — accept false negatives over false positives
- [ ] Remove or update test `i128_token_id_not_excluded` — it currently asserts the broken
      behavior; replace with a test that verifies WASM-classified NFT contracts with i128
      token IDs are still detected correctly
- [ ] Existing NFT detection tests still pass (for string/bytes token_id contracts)
- [ ] New test: SEP-0041 fungible transfer event does NOT produce an NFT record
- [ ] New test: NFT contract (WASM-classified) with i128 token_id still detected correctly
- [ ] New test: unknown contract (no WASM metadata) with i128 data does NOT produce NFT record
