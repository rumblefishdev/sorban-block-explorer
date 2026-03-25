---
id: '0063'
title: 'XDR parsing: LedgerEntryChanges extraction'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0060', '0019', '0020']
tags: [priority-medium, effort-medium, layer-indexing]
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# XDR parsing: LedgerEntryChanges extraction

## Summary

Implement extraction of derived explorer state from LedgerEntryChanges within each transaction's result metadata. This task covers contract deployments (including SAC detection), account state updates, liquidity pool state and snapshots, token detection, and NFT detection. These are the "derived-state" entities that the explorer builds from raw ledger mutations to support contract, account, token, NFT, and pool views.

## Status: Backlog

**Current state:** Not started. Depends on task 0060 for parsed LedgerCloseMeta data. Database schema tasks 0019 (tokens, accounts) and 0020 (NFTs, pools, snapshots) define the target tables.

## Context

LedgerEntryChanges represent the low-level mutations applied to the Stellar ledger during transaction execution. The explorer does not expose raw ledger entries directly. Instead, it translates these mutations into query-oriented derived entities: contracts, accounts, tokens, NFTs, and liquidity pools.

This is the final parsing stage in the pipeline (after 0060, 0061, 0062). It consumes the parsed LedgerCloseMeta and NFT-related events emitted by task 0062, then upserts derived-state tables.

All derived-state writes use watermark-based upserts (task 0065) to ensure backfill data does not overwrite newer live-derived state.

### Source Code Location

- `apps/indexer/src/parsers/entry-changes/`

## Implementation Plan

### Step 1: Contract Deployment Extraction

From LedgerEntryChanges of contract type, extract:

- `contract_id`: the deployed contract address
- `wasm_hash`: hash of the contract WASM
- `deployer_account`: the account that deployed the contract
- `deployed_at_ledger`: the current ledger sequence number
- `contract_type`: classify as 'token', 'dex', 'lending', 'nft', or 'other' based on deployment patterns and WASM analysis
- `is_sac`: boolean, true if this is a Stellar Asset Contract (SAC). Detect SACs from deployment patterns (SACs wrap classic Stellar assets as Soroban contracts).
- `metadata`: initialize JSONB metadata object. Interface data may be populated by task 0062's contract interface extraction.

Write to `soroban_contracts` table. Upsert on `contract_id`: deployment fields on first insert, metadata updated when interface extraction completes (from task 0062).

### Step 2: Account State Extraction

From LedgerEntryChanges of account type, extract and upsert to the `accounts` table:

- `account_id` (PK): the Stellar account address
- `first_seen_ledger`: set on account creation (first time this account appears). Do not overwrite on subsequent changes.
- `last_seen_ledger`: update on every change to this account. This is the watermark column -- only apply if incoming ledger_sequence >= current last_seen_ledger.
- `sequence_number`: current account sequence number
- `balances`: JSONB array of account balances (native + trustline balances)
- `home_domain`: account home domain if set

### Step 3: Liquidity Pool State Extraction

From LedgerEntryChanges of liquidity pool type, extract and upsert to the `liquidity_pools` table:

- `pool_id` (PK): the liquidity pool identifier
- `asset_a`: JSONB describing the first pool asset
- `asset_b`: JSONB describing the second pool asset
- `fee_bps`: pool fee in basis points
- `reserves`: JSONB with current reserve amounts
- `total_shares`: total LP shares outstanding
- `tvl`: total value locked, if derivable from available data
- `created_at_ledger`: set on first appearance of this pool. Do not overwrite.
- `last_updated_ledger`: update on every change. This is the watermark column.

### Step 4: Liquidity Pool Snapshot Append

On each pool change detected in Step 3, append a snapshot row to `liquidity_pool_snapshots`:

- `pool_id` FK
- `ledger_sequence`: current ledger
- `created_at`: from ledger closeTime
- `reserves`: current reserves at this point
- `total_shares`: current total shares
- `tvl`: current TVL if available
- `volume`: trading volume if derivable
- `fee_revenue`: fee revenue if derivable

This is APPEND-ONLY. Snapshot rows are never updated or deleted (except via partition management). They provide the time-series data for pool chart endpoints.

### Step 5: Token Detection

Populate the `tokens` table when token contracts or classic assets are detected:

- `asset_type`: 'classic', 'sac', or 'soroban' depending on the token origin
- `asset_code`: for classic assets and SACs
- `issuer_address`: for classic assets
- `contract_id`: for Soroban token contracts (references soroban_contracts)
- `name`: token name if available from metadata
- `total_supply`: if derivable from ledger state
- `holder_count`: if derivable from ledger state

Token detection combines information from contract deployments (Step 1), SAC detection, and known token contract patterns.

### Step 6: NFT Detection

Populate the `nfts` table using a combination of LedgerEntryChanges and NFT-related events emitted by task 0062:

- `contract_id`: the NFT contract (references soroban_contracts)
- `token_id`: unique token identifier within the collection
- `collection_name`: collection name if available
- `owner_account`: current owner address
- `name`: NFT name if available
- `media_url`: media URL if available from metadata
- `metadata`: JSONB with additional NFT metadata
- `minted_at_ledger`: set when the NFT is first minted
- `last_seen_ledger`: update on every NFT state change. This is the watermark column.

NFT ownership transfers are detected from transfer events. Metadata updates come from contract state changes.

### Step 7: SAC Detection

Stellar Asset Contracts (SACs) are detected from deployment patterns:

- SACs wrap classic Stellar assets as Soroban contracts
- Detection uses the deployer pattern and contract initialization data
- When a SAC is detected, set `is_sac = true` on the soroban_contracts row
- Link the SAC to the corresponding classic asset in the tokens table if applicable

## Acceptance Criteria

- [ ] Contract deployments are extracted with contract_id, wasm_hash, deployer_account, deployed_at_ledger, contract_type, is_sac, and initial metadata
- [ ] SACs are correctly identified and flagged with is_sac=true
- [ ] Account state is upserted with account_id PK, first_seen_ledger (on creation only), last_seen_ledger (on every change), sequence_number, balances, home_domain
- [ ] Liquidity pool state is upserted with pool_id PK, asset_a, asset_b, fee_bps, reserves, total_shares, tvl, created_at_ledger (on first), last_updated_ledger (on change)
- [ ] Liquidity pool snapshots are appended (never updated) on each pool change with ledger_sequence, created_at, reserves, total_shares, tvl, volume, fee_revenue
- [ ] Tokens are detected and populated with asset_type, asset_code, issuer_address or contract_id, name, total_supply, holder_count
- [ ] NFTs are detected and populated with contract_id, token_id, collection_name, owner_account, name, media_url, metadata, minted_at_ledger, last_seen_ledger
- [ ] All derived-state upserts respect watermark columns (last_seen_ledger, last_updated_ledger) to prevent stale backfill overwrites
- [ ] Unit tests cover each entity extraction path, watermark enforcement, SAC detection, and NFT event consumption

## Notes

- Contract type classification ('token', 'dex', 'lending', 'nft', 'other') may require heuristics and pattern matching. Start with known patterns and classify unknown contracts as 'other'.
- NFT contract conventions on Stellar/Soroban are still evolving. The detection logic should be extensible. Research task 0005 documents known patterns.
- Liquidity pool TVL, volume, and fee_revenue may not always be derivable from ledger state alone. Store null when not available.
- The append-only nature of liquidity_pool_snapshots means this table grows proportionally to pool activity. Monthly partitioning on created_at keeps maintenance practical.
- soroban_contracts upsert must handle the case where task 0062's interface extraction runs before or after the deployment extraction in this task. Both paths must converge on the same contract row.
