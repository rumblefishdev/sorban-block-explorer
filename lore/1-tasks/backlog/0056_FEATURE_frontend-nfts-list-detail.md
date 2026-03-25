---
id: '0056'
title: 'Frontend: NFTs list and detail pages'
type: FEATURE
status: backlog
related_adr: []
related_tasks: []
tags: [priority-medium, effort-medium, layer-frontend-pages]
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# Frontend: NFTs list and detail pages

## Summary

Implement the NFTs list page (`/nfts`) and NFT detail page (`/nfts/:id`). Supports browsing Soroban-based NFT contracts with media previews, metadata attributes, and transfer history. Graceful degradation for broken media and sparse metadata.

## Status: Backlog

**Current state:** Not started.

## Context

NFTs on Stellar are Soroban-based. The NFT pages prioritize recognition and collection browsing over raw protocol detail. Media assets may fail to load, metadata may be incomplete or irregular. The frontend must tolerate all of this gracefully.

### API Endpoints Consumed

| Endpoint                  | Query Params                                                   | Purpose                                                                  |
| ------------------------- | -------------------------------------------------------------- | ------------------------------------------------------------------------ |
| `GET /nfts`               | `limit`, `cursor`, `filter[collection]`, `filter[contract_id]` | Paginated NFT list with filters                                          |
| `GET /nfts/:id`           | none                                                           | NFT detail: name, token ID, collection, contract, owner, metadata, media |
| `GET /nfts/:id/transfers` | `limit`, `cursor`                                              | Paginated transfer history for this NFT                                  |

### NFT List Table/Grid Columns

| Column        | Display                               | Notes                                     |
| ------------- | ------------------------------------- | ----------------------------------------- |
| Name          | Text                                  | NFT name or identifier                    |
| Collection    | Text                                  | Collection name                           |
| Contract      | Truncated, linked to `/contracts/:id` | IdentifierDisplay (task 0042)             |
| Owner         | Truncated, linked to `/accounts/:id`  | IdentifierDisplay (task 0042)             |
| Preview Image | Thumbnail                             | Lazy-loaded. Placeholder on load failure. |

### NFT List Filters

| Filter      | Type                  | Notes                            |
| ----------- | --------------------- | -------------------------------- |
| Collection  | Text input / dropdown | Filters by `filter[collection]`  |
| Contract ID | Text input            | Filters by `filter[contract_id]` |

- Filters reflected in URL, filter change resets cursor

### NFT Detail Fields

| Field      | Display                          | Notes                              |
| ---------- | -------------------------------- | ---------------------------------- |
| Name       | Prominent header                 | NFT name                           |
| Token ID   | Text                             | NFT identifier within the contract |
| Collection | Text                             | Collection name                    |
| Contract   | Full, linked to `/contracts/:id` | IdentifierWithCopy (task 0042)     |
| Owner      | Full, linked to `/accounts/:id`  | IdentifierWithCopy (task 0042)     |

### Media Preview

- Supports image and video formats
- Graceful degradation: broken URLs show placeholder image
- Unsupported formats show placeholder with format label
- Lazy-loaded to avoid blocking page render

### Metadata Attributes

- Full attribute list (traits, properties)
- Rendered as key-value pairs or tag grid
- Must remain readable even when metadata is deep or irregular
- Tolerates sparse metadata (missing fields shown as "N/A" or omitted gracefully)

### Transfer History Table Columns

| Column           | Display                                    | Notes                         |
| ---------------- | ------------------------------------------ | ----------------------------- |
| From             | Truncated, linked to `/accounts/:id`       | IdentifierDisplay (task 0042) |
| To               | Truncated, linked to `/accounts/:id`       | IdentifierDisplay (task 0042) |
| Ledger           | Linked to `/ledgers/:sequence`             | IdentifierDisplay (task 0042) |
| Timestamp        | Relative                                   | RelativeTimestamp (task 0043) |
| Transaction Hash | Truncated, linked to `/transactions/:hash` | IdentifierDisplay (task 0042) |

- Paginated from `/nfts/:id/transfers` endpoint

## Implementation Plan

### Step 1: NFT list query hook and page

Create `apps/web/src/pages/nfts/useNFTsList.ts` and `NFTsListPage.tsx`:

- Fetches `GET /nfts` with filters and cursor
- Filter controls: collection, contract ID
- Table/grid with: name, collection, contract (linked), owner (linked), preview image
- Preview images lazy-loaded with placeholder fallback

### Step 2: NFT detail query hooks

Create `apps/web/src/pages/nft-detail/useNFTDetail.ts` and `useNFTTransfers.ts`:

- `useNFTDetail`: fetches `GET /nfts/:id`, stale time 5 minutes
- `useNFTTransfers`: fetches `GET /nfts/:id/transfers` with cursor

### Step 3: NFT detail summary and media

Create `apps/web/src/pages/nft-detail/NFTSummary.tsx` and `NFTMediaPreview.tsx`:

- Summary: name, token ID, collection, contract (linked), owner (linked)
- Media: image/video with graceful degradation, placeholder on failure

### Step 4: Metadata attributes section

Create `apps/web/src/pages/nft-detail/NFTMetadata.tsx`:

- Renders attributes as key-value pairs or tag grid
- Handles deep/irregular metadata structures
- Sparse tolerance: missing values handled gracefully

### Step 5: Transfer history section

Create `apps/web/src/pages/nft-detail/NFTTransfers.tsx`:

- Paginated table: from (linked), to (linked), ledger (linked), timestamp, tx hash (linked)
- SectionHeader: "Transfer History"

### Step 6: Page composition

Create `apps/web/src/pages/nft-detail/NFTDetailPage.tsx`:

- Composes: NFTSummary, NFTMediaPreview, NFTMetadata, NFTTransfers
- Each section in SectionErrorBoundary (task 0044)
- 404 state: "NFT not found"

## Acceptance Criteria

- [ ] NFT list shows: name, collection, contract (linked), owner (linked), preview image (lazy-loaded)
- [ ] Preview images: placeholder on load failure, lazy-loaded
- [ ] List filters: collection, contract ID. Reflected in URL.
- [ ] Detail shows: name, token ID, collection, contract (linked, copyable), owner (linked, copyable)
- [ ] Media preview supports image/video with graceful degradation for broken URLs
- [ ] Metadata attributes readable even when deep/irregular; sparse data handled gracefully
- [ ] Transfer history: paginated table with from, to, ledger, timestamp, tx hash (all linked)
- [ ] 404 state: "NFT not found"
- [ ] Loading skeleton and error states per section

## Notes

- NFT metadata quality varies significantly across the ecosystem. The UI must never break on unexpected metadata shapes.
- Contract links allow users to move from NFT browsing into contract inspection.
- Preview images should not block page usability -- load them asynchronously with placeholders.
