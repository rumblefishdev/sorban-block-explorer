---
id: '0005'
title: 'Research: Soroban NFT ecosystem patterns and detection heuristics'
type: RESEARCH
status: backlog
related_adr: []
related_tasks: ['0055', '0012']
tags: [priority-medium, effort-medium, layer-research]
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created from architecture docs decomposition'
---

# Research: Soroban NFT ecosystem patterns and detection heuristics

## Summary

Investigate the current state of NFT conventions on the Soroban network, including contract interface patterns, event signatures for mint/transfer/burn, metadata standards, and detection heuristics. This research must determine how the block explorer can identify, index, and display NFTs given that Soroban has no ERC-721 equivalent standard.

## Status: Backlog

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

- [ ] Survey of current Soroban NFT conventions and any emerging standards
- [ ] Documented function signature patterns for NFT contract detection
- [ ] Documented event patterns for mint, transfer, and burn with topic/data structure
- [ ] Detection strategy recommendation: deployment-time, event-time, or both
- [ ] Token ID extraction method documented
- [ ] Metadata retrieval approach documented (on-chain, off-chain, hybrid)
- [ ] List of known mainnet NFT contracts for testing (if any exist)
- [ ] Sparse metadata handling guidelines for the frontend display layer
- [ ] False positive assessment and mitigation strategy

## Notes

- The frontend NFT list page shows: name/identifier, collection name, contract ID, owner, and preview image. The detail page adds: media preview, full attribute list, and transfer history. All of these fields must be sourceable from the detection and extraction approach chosen.
- The `soroban_contracts` table has a `search_vector` on `metadata->>'name'`, so NFT contract names must be stored in the metadata JSONB for search discoverability.
- NFT transfer history is reconstructed from `soroban_events` joined to the NFT's contract_id. The event topics GIN index supports this query pattern.
