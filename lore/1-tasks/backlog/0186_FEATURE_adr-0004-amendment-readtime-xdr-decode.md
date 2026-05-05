---
id: '0186'
title: 'Docs: ADR 0004 amendment for read-time XDR decode of S3-fetched archive payloads'
type: FEATURE
status: backlog
related_adr: ['0004', '0033', '0034']
related_tasks: ['0123', '0122', '0046', '0050', '0150']
tags: [priority-low, effort-small, layer-docs, follow-up]
links: []
history:
  - date: '2026-05-04'
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from 0123 closure (superseded by 0150 + 0046 + ADRs
      0033/0034). ADR 0004 ("all XDR parsing in Rust at ingest time,
      API is pure CRUD") is no longer fully accurate — read-time
      decode of S3-fetched envelope_xdr/result_xdr/result_meta_xdr
      is part of the architecture (ADRs 0033/0034 partially supersede
      it). This task formalizes the amendment so future contributors
      have a single source of truth. Same amendment forward-covers the
      read-time signatures path under consideration for task 0122.
---

# Docs: ADR 0004 amendment for read-time XDR decode of S3-fetched archive payloads

## Summary

Amend ADR 0004 (or add an addendum) to reflect the read-time XDR decode pattern shipped via tasks 0150 / 0046 / 0050 and supported by ADRs 0033 / 0034. Forward-cover the signatures path being considered under task 0122 so it does not require a second amendment.

## Context

ADR 0004 currently states "all XDR parsing happens exclusively in Rust at ingestion time" and the API is "pure CRUD." Subsequent decisions diverged:

- **0150** (archived 2026-04-22) — `crates/api/src/stellar_archive` fetches `envelope_xdr/result_xdr/result_meta_xdr` from the public Stellar archive at request time and decodes them to surface heavy fields.
- **0046** (archived 2026-04-23) — Wired heavy fields into `GET /v1/transactions/:hash`.
- **ADR 0033** — Read-time XDR decode for Soroban events appearances (E14).
- **ADR 0034** — Read-time XDR decode for Soroban invocations appearances (E3 advanced).

These do not technically violate "all parsing in Rust" — the decode is still Rust, just at request time. But the "ingestion time only" wording is wrong for heavy fields. ADR 0004 should be updated so future contributors don't replicate scoping mistakes (the duplication of 0123 over 0150 is the concrete cost of the gap).

## Acceptance Criteria

- [ ] Decision recorded: amend ADR 0004 in place vs add `0004a_*` addendum (per ADR 0032 evergreen policy)
- [ ] Allowed read-time decode pattern documented: only XDR fetched on demand from the public Stellar archive (S3); no decode of stored DB columns at request time
- [ ] Cross-reference ADRs 0033 and 0034 as the events/invocations applications of the pattern
- [ ] Cross-reference task 0122 (signatures) — once that task picks its approach, the amendment links it (or vice versa)
- [ ] `docs/architecture/xdr-parsing-overview.md` and `docs/architecture/backend-overview.md` updated to reflect the wording change (per ADR 0032)

## Implementation

Pure docs change — no code, no schema, no infrastructure. Single PR touching:

1. `lore/2-adrs/0004_*.md` (or new `0004a_*.md` addendum, per chosen format)
2. Cross-references in `docs/architecture/xdr-parsing-overview.md` and `docs/architecture/backend-overview.md`
3. `lore/2-adrs/0033_*.md` and `0034_*.md` — update relationship to ADR 0004 if "supersedes/amends" link is missing

## Notes

Out of scope:

- Drafting of the actual amendment wording (handled at pickup time).
- Decision about task 0122 (signatures) approach — that task picks its path independently; this task only ensures the doc state captures whatever it lands on.
