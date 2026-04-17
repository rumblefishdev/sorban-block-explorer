---
id: '0122'
title: 'Indexer: extract transaction signatures'
type: FEATURE
status: backlog
related_adr: ['0012']
related_tasks: ['0024', '0046', '0140', '0142']
blocked_by: ['0142']
tags:
  [
    priority-low,
    effort-small,
    layer-indexer,
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
    note: 'Spawned from pipeline audit — tech design specifies signatures display on tx detail but XDR parser does not extract them.'
  - date: '2026-04-17'
    status: backlog
    who: stkrolikiewicz
    note: >
      Audit per task 0140 — ADR 0012 supersedes the underlying schema/flow
      patterns referenced in body. Blocked by 0142 (schema migration). Body
      must be re-read against ADR 0012 before implementing.
---

> **⚠ Post-ADR 0012 re-read required (audit 2026-04-17, [task 0140](../active/0140_DOCS_audit-lore-tasks-adr-0011-0012.md)):**
> Body below references pre-ADR-0012 patterns (schema / flow / JSONB / upsert / `transaction_id` partitioning). [ADR 0012](../../2-adrs/0012_zero-upsert-schema-full-fk-graph.md) supersedes with zero-upsert history tables, activity projections, S3 offload, and `created_at` partitioning. Blocked by 0142 (schema migration) — do not implement until migration lands and this task is re-aligned.

---

# Indexer: extract transaction signatures

## Summary

The technical design specifies showing signature data on the transaction detail page.
`DecoratedSignature` in Stellar XDR contains only a 4-byte public key hint and the
signature blob — signer weight is NOT available from the envelope (it lives in the
account's signers list on the ledger). The XDR parser does not extract signatures and
the transactions table has no signatures column.

## Implementation

1. Extract `signatures` from `TransactionEnvelope` during XDR parsing (they are in the
   envelope's `signatures` field — `Vec<DecoratedSignature>`).
2. Store as JSONB column on `transactions` table or decode from `envelope_xdr` at API time.
3. Recommendation: store at ingestion time (consistent with ADR 0004 — no server-side XDR).

## Acceptance Criteria

- [ ] Transaction signatures extracted and stored (JSONB array)
- [ ] Each signature includes: public key hint, signature hex
- [ ] API returns signatures in transaction detail response
