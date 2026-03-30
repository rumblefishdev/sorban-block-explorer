---
id: '0005'
title: 'Research: Soroban NFT ecosystem patterns and detection heuristics'
type: RESEARCH
status: completed
assignee: fmazur
related_adr: []
related_tasks: ['0076', '0012']
tags: [priority-medium, effort-medium, layer-research]
milestone: 1
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created from architecture docs decomposition'
  - date: 2026-03-26
    status: active
    who: fmazur
    note: 'Promoted to active'
  - date: 2026-03-27
    status: completed
    who: fmazur
    note: >
      Research complete. 5 notes (3 R-, 1 S-, 1 G-), 11 sources archived.
      Key findings: SEP-0050 is the emerging NFT standard, dual detection
      strategy recommended (WASM spec primary + event confirmation secondary),
      1 known mainnet contract (jamesbachini). All 9 acceptance criteria met.
---

# Research: Soroban NFT ecosystem patterns and detection heuristics

## Summary

Investigate the current state of NFT conventions on the Soroban network, including contract interface patterns, event signatures for mint/transfer/burn, metadata standards, and detection heuristics. This research must determine how the block explorer can identify, index, and display NFTs given that Soroban has no ERC-721 equivalent standard.

## Status: Completed

## Context

There is no ERC-721 equivalent on Soroban. The NFT ecosystem is nascent and conventions are still forming. The block explorer must still support NFT browsing, detail views, and transfer history as defined in the product scope, which means the indexing pipeline needs heuristics to identify NFT contracts and extract meaningful data from them.

### NFT Table Schema

The `nfts` table is designed to hold the following fields that must be populated by the indexing pipeline:

- `contract_id` (VARCHAR 56, FK to soroban_contracts) -- the contract that manages this NFT
- `token_id` (VARCHAR 128) -- unique within the contract
- `collection_name` (VARCHAR 100) -- optional, if the contract organizes tokens into collections
- `owner_account` (VARCHAR 56) -- current owner
- `name` (VARCHAR 100) -- human-readable name if available
- `media_url` (TEXT) -- link to media asset if available
- `metadata` (JSONB) -- full attribute list (traits, properties)
- `minted_at_ledger` (BIGINT, FK to ledgers) -- when the NFT was first seen
- `last_seen_ledger` (BIGINT, FK to ledgers) -- most recent activity

Uniqueness is scoped by `(contract_id, token_id)`.

### Transfer History

NFT transfer history is derived from `soroban_events`, not stored in a separate table. This means the event indexing must capture transfer events in a way that allows the `GET /nfts/:id/transfers` endpoint to reconstruct ownership history from event records.

### Detection Timing

NFT detection could happen at multiple points in the pipeline:

1. **Deployment time** -- WASM analysis of the contract interface to identify NFT-like function signatures
2. **Event time** -- Observing mint events or transfer events that follow NFT patterns
3. **Both** -- Combining deployment-time classification with runtime event confirmation

The research must recommend which approach (or combination) is most reliable given the current ecosystem state.

### Sparse Metadata Tolerance

The architecture explicitly expects sparse metadata tolerance. NFT contract conventions vary heavily, and many NFTs may have incomplete or inconsistent descriptive fields. The `metadata` and `media_url` columns are optional. The explorer must handle NFTs with missing names, missing images, and partial attribute lists gracefully.

### Contract Type Classification

NFT contracts are one of the `contract_type` categories ('nft') in the `soroban_contracts` table. The WASM interface extraction research (task 0003) addresses classification broadly, but this task focuses specifically on NFT-related patterns and detection confidence.

## Research Questions

- What are the current NFT contract conventions on Soroban? Are there any emerging standards or SEPs (Stellar Ecosystem Proposals) for NFTs?
- What function signatures characterize an NFT contract (e.g., mint, transfer, burn, balance_of, token_uri, owner_of)?
- What event patterns do NFT contracts emit for mint, transfer, and burn operations? What do the event topics and data fields contain?
- Are there known NFT contracts deployed on Soroban mainnet that can serve as reference implementations?
- How should `token_id` be extracted from contract events or invocations? Is it typically a numeric ID, a string, or some other ScVal type?
- How can `collection_name` be determined? Is it typically stored in contract storage, or passed as a constructor argument?
- What metadata standards exist for Soroban NFTs? Is metadata typically on-chain (in contract storage), off-chain (IPFS/HTTP URL), or a hybrid?
- How reliable is deployment-time WASM detection vs event-time pattern matching for identifying NFT contracts?
- What is the expected false positive rate for NFT detection heuristics given the current ecosystem?
- How should the explorer handle contracts that appear NFT-like but do not conform to any known pattern?

## Acceptance Criteria

- [x] Survey of current Soroban NFT conventions and any emerging standards — `notes/R-soroban-nft-standards.md`
- [x] Documented function signature patterns for NFT contract detection — `notes/R-wasm-spec-detection.md`
- [x] Documented event patterns for mint, transfer, and burn with topic/data structure — `notes/R-event-patterns-detection.md`
- [x] Detection strategy recommendation: deployment-time, event-time, or both — `notes/S-detection-strategy-recommendation.md`
- [x] Token ID extraction method documented — `notes/S-detection-strategy-recommendation.md`
- [x] Metadata retrieval approach documented (on-chain, off-chain, hybrid) — `notes/S-detection-strategy-recommendation.md`
- [x] List of known mainnet NFT contracts for testing (if any exist) — `notes/G-known-nft-contracts.md`
- [x] Sparse metadata handling guidelines for the frontend display layer — `notes/S-detection-strategy-recommendation.md`
- [x] False positive assessment and mitigation strategy — `notes/S-detection-strategy-recommendation.md`

## Implementation Notes

**Research artifacts produced:**

| Note                                     | Type       | Content                                                                                             |
| ---------------------------------------- | ---------- | --------------------------------------------------------------------------------------------------- |
| `R-soroban-nft-standards.md`             | Research   | SEP-0050, SEP-0041, SEP-0039 survey + OpenZeppelin implementation                                   |
| `R-event-patterns-detection.md`          | Research   | Event topic/data structures, NFT vs fungible differentiation                                        |
| `R-wasm-spec-detection.md`               | Research   | WASM contractspecv0 analysis, detection heuristic with confidence scoring                           |
| `S-detection-strategy-recommendation.md` | Synthesis  | Dual detection strategy, token ID extraction, metadata retrieval, sparse tolerance, false positives |
| `G-known-nft-contracts.md`               | Generation | Known mainnet/testnet contracts for testing                                                         |

**Sources archived:** 11 files in `sources/` covering SEP-0050, SEP-0041, SEP-0039, OpenZeppelin docs, Stellar events/RPC docs, and jamesbachini tutorial with contract code.

## Design Decisions

### From Plan

1. **SEP-0050 as primary detection target**: The task context identified SEP-0050 as the emerging standard. Research confirmed it is the only formal Soroban NFT standard (Draft, v0.1.0).

2. **Dual detection (WASM + events)**: The task asked to recommend deployment-time, event-time, or both. Recommended both with WASM as primary and events as secondary confirmation.

### Emerged

3. **Confidence-based scoring over binary classification**: Rather than a yes/no NFT classification, introduced a 4-level confidence scale (High/Medium/Low/None) to handle the nascent ecosystem where many contracts won't fully conform to SEP-0050.

4. **Case-insensitive event matching**: Discovered that the only known mainnet NFT contract (jamesbachini) uses capitalized event symbols (`"Transfer"` vs `"transfer"`), requiring case-insensitive matching in the event filter.

5. **`decimals()` as the key discriminator**: Identified that the presence of `decimals()` is the strongest signal to exclude fungible tokens from NFT classification, since SEP-0050 explicitly omits it while SEP-0041 requires it.

## Issues Encountered

- **No standardized burn event in SEP-0050**: The core `NonFungibleToken` trait has no burn function or event. Only the OpenZeppelin `NonFungibleBurnable` extension adds it. Burn detection must rely on WASM spec analysis rather than events.
- **TokenID type variance**: SEP-0050 defines `TokenID` as a generic unsigned integer. OpenZeppelin uses `u32`, but the only known mainnet contract uses `i128`. The `VARCHAR(128)` storage was already designed for this, but the detection heuristic must handle both types.

## Notes

- The frontend NFT list page shows: name/identifier, collection name, contract ID, owner, and preview image. The detail page adds: media preview, full attribute list, and transfer history. All of these fields must be sourceable from the detection and extraction approach chosen.
- The `soroban_contracts` table has a `search_vector` on `metadata->>'name'`, so NFT contract names must be stored in the metadata JSONB for search discoverability.
- NFT transfer history is reconstructed from `soroban_events` joined to the NFT's contract_id. The event topics GIN index supports this query pattern.
