---
id: '0122'
title: 'Indexer: extract transaction signatures'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0024', '0046']
tags: [priority-low, effort-small, layer-indexer, audit-gap]
milestone: 1
links:
  - docs/audits/2026-04-10-pipeline-data-audit.md
history:
  - date: '2026-04-10'
    status: backlog
    who: stkrolikiewicz
    note: 'Spawned from pipeline audit — tech design specifies signatures display on tx detail but XDR parser does not extract them.'
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
