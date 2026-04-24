---
id: '0121'
title: 'NFT transfer history: schema + API endpoint'
type: FEATURE
status: blocked
related_adr: ['0037']
related_tasks: ['0051', '0118', '0136']
blocked_by: ['0118', '0136']
tags: [priority-medium, effort-medium, layer-backend, layer-db, audit-gap]
milestone: 1
links:
  - docs/audits/2026-04-10-pipeline-data-audit.md
history:
  - date: '2026-04-10'
    status: backlog
    who: stkrolikiewicz
    note: 'Spawned from pipeline audit — tech design requires GET /nfts/:id/transfers but no schema exists.'
---

# NFT transfer history: schema + API endpoint

## Summary

The technical design specifies `GET /nfts/:id/transfers` and a "Transfer history" section
on the NFT detail page, but no `nft_transfers` table exists in the schema. The `nfts` table
only stores current owner — transfer history is lost.

## Implementation

Option A: Create an `nft_transfers` table populated during indexing from mint/transfer/burn
events.

Option B: Query `soroban_events` filtered by NFT contract + transfer topic pattern at API
query time (no new table, but slower and requires careful index design).

Recommendation: Option A — dedicated table with proper indexes for fast history queries.

**Blocker:** Task 0118 (NFT false positive fix) must be completed first, otherwise the
transfer history table will also be flooded with spurious fungible transfer entries.

## Acceptance Criteria

- [ ] NFT transfer history queryable by contract_id + token_id
- [ ] Each transfer records: from, to, ledger_sequence, timestamp, event_type (mint/transfer/burn)
- [ ] API endpoint `GET /nfts/:id/transfers` returns paginated transfer history
- [ ] Indexer populates transfer records during event processing
