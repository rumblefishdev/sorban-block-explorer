---
id: '0123'
title: 'API: XDR decoding service for advanced transaction view'
type: FEATURE
status: active
related_adr: ['0004']
related_tasks: ['0046', '0071']
tags: [priority-medium, effort-medium, layer-backend, audit-gap]
milestone: 1
links:
  - docs/audits/2026-04-10-pipeline-data-audit.md
history:
  - date: '2026-04-10'
    status: backlog
    who: stkrolikiewicz
    note: 'Spawned from pipeline audit — tech design allocates 4 days for XDR decode service but no task existed.'
  - date: '2026-04-24'
    status: active
    who: kkowalczyk
    note: 'Promoted to active to start work.'
---

# API: XDR decoding service for advanced transaction view

## Summary

The technical design allocates 4 estimated days for an on-demand XDR decoding capability
at the API layer. The frontend advanced transaction view (task 0071) depends on this to
show decoded `envelope_xdr`, `result_xdr`, and `result_meta_xdr`.

## Context

ADR 0004 states "all XDR parsing happens in Rust at ingestion time" and the API is "pure
CRUD." However, the advanced view needs to show decoded XDR structures that are NOT
pre-materialized. Two options:

1. Decode at ingestion time and store decoded forms (storage cost, but consistent with ADR).
2. Add an on-demand decode endpoint (violates ADR 0004 spirit, but avoids schema bloat).

## Acceptance Criteria

- [ ] Raw XDR (envelope, result, result_meta) can be decoded to structured JSON
- [ ] Frontend advanced view can display decoded XDR sections
- [ ] Collapsible sections for large payloads per tech design spec
- [ ] ADR 0004 amended or addendum created to document the chosen approach and rationale
