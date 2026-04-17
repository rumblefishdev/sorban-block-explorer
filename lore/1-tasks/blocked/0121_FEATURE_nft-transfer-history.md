---
id: '0121'
title: 'NFT transfer history: schema + API endpoint'
type: FEATURE
status: blocked
related_adr: ['0012']
related_tasks: ['0051', '0118', '0136', '0140', '0142']
blocked_by: ['0118', '0136', '0142']
tags:
  [
    priority-medium,
    effort-medium,
    layer-backend,
    layer-db,
    audit-gap,
    pending-adr-0012-rewrite,
  ]
milestone: 1
links:
  - docs/audits/2026-04-10-pipeline-data-audit.md
history:
  - date: '2026-04-10'
    status: backlog
    who: stkrolikiewicz
    note: 'Spawned from pipeline audit — tech design requires GET /nfts/:id/transfers but no schema exists.'
  - date: '2026-04-17'
    status: blocked
    who: stkrolikiewicz
    note: >
      Audit per task 0140 — ADR 0012 supersedes the underlying schema/flow
      patterns referenced in body. Blocked by 0142 (schema migration). Body
      must be re-read against ADR 0012 before implementing.
---

> **⚠ Post-ADR 0012 re-read required (audit 2026-04-17, [task 0140](../active/0140_DOCS_audit-lore-tasks-adr-0011-0012.md)):**
> Body below references pre-ADR-0012 patterns (schema / flow / JSONB / upsert / `transaction_id` partitioning). [ADR 0012](../../2-adrs/0012_zero-upsert-schema-full-fk-graph.md) supersedes with zero-upsert history tables, activity projections, S3 offload, and `created_at` partitioning. Blocked by 0142 (schema migration) — do not implement until migration lands and this task is re-aligned.

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
