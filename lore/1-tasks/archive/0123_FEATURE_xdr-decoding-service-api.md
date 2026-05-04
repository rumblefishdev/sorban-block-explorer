---
id: '0123'
title: 'API: XDR decoding service for advanced transaction view'
type: FEATURE
status: archive
related_adr: ['0004', '0029', '0033', '0034']
related_tasks: ['0046', '0050', '0071', '0150', '0186']
superseded_by: ['0150', '0046', '0033', '0034']
tags: [priority-medium, effort-medium, layer-backend, audit-gap, superseded]
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
    who: karolkow
    note: 'Promoted to active to start work.'
  - date: '2026-04-24'
    status: backlog
    who: karolkow
    note: >
      Demoted after investigation — backend XDR decode already delivered
      by task 0150 (stellar_archive library) + task 0046 (E3 wiring).
      Real remaining work = ADR 0004 amendment/supersession (AC#4).
      Likely duplicate of 0150. Paused pending PM (stkrolikiewicz) sync
      on scope: cancel as duplicate, reduce scope to ADR 0004 update only,
      or close as completed with delegated credit. See related_tasks 0150
      (completed 2026-04-22) and 0046 (completed 2026-04-23). Meanwhile
      switched to 0043 per team sync.
  - date: '2026-05-04'
    status: archive
    who: stkrolikiewicz
    note: >
      Closed as superseded. PM sync confirmed: scope fully absorbed by
      task 0150 (`crates/api/src/stellar_archive` lib, archived 2026-04-22),
      task 0046 (E3 wiring of heavy fields, archived 2026-04-23), and
      ADRs 0033 + 0034 (read-time XDR decode for events/invocations
      from S3-fetched archive). AC#1-3 delivered via those tasks; AC#4
      (ADR 0004 amendment) spawned as task 0186. No code work remained.
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

- [x] Raw XDR (envelope, result, result_meta) can be decoded to structured JSON — covered by task 0150 (`crates/api/src/stellar_archive` library) and ADRs 0033/0034
- [x] Frontend advanced view can display decoded XDR sections — task 0046 (E3) wires `heavy.envelope_xdr/result_xdr/result_meta_xdr/operation_tree/contract_events`
- [x] Collapsible sections for large payloads per tech design spec — frontend concern; contract supports it via the heavy fields response shape
- [ ] ADR 0004 amended or addendum created to document the chosen approach and rationale — deferred to task 0186

## Implementation Notes

No code or schema work was performed under this task ID. Investigation on 2026-04-24 revealed the scope had already shipped under sibling tasks before this one was activated:

- **0150** (archived 2026-04-22) — `StellarArchiveFetcher`, S3 key construction, heavy-field DTOs, per-endpoint extractors, merge functions. Unsigned S3 client (us-east-2). 12 unit + 6 integration tests passing.
- **0046** (archived 2026-04-23) — Wired heavy fields into `GET /v1/transactions/:hash`. Tested against ledger 62248883.
- **ADR 0033** — Read-time XDR decode for Soroban events appearances (E14).
- **ADR 0034** — Read-time XDR decode for Soroban invocations appearances (E3 advanced).

The two options listed in Context resolved as: option 2 (on-demand decode) won, but applied per-endpoint via heavy fields fetched on demand from the public Stellar archive, not via a centralized "decoding service" endpoint.

## Design Decisions

### From Plan

1. **Closed as superseded rather than re-scoped to ADR amendment**: The task was created as a 4-day effort assuming greenfield XDR decode infrastructure. Reusing the task for a single-AC ADR amendment would inflate effort tracking and confuse future sessions; spawning a fresh narrow task (0186) is cleaner.

### Emerged

2. **Recognition lag for supersession**: Tasks 0150 and 0046 were created and merged independently in April 2026 without explicitly closing 0123. The duplication only surfaced during a re-audit on 2026-05-04. Cause: task ACs overlapped with recently-archived sibling tasks; no duplicate-check pass on activation.

3. **AC#4 deferred to dedicated follow-up task 0186**: ADR 0004 has not been formally amended despite ADRs 0033/0034 partially superseding its "no read-time decode" stance in practice. Rather than embedding the amendment into this closed task, spawned a single-AC task so the doc state is explicit and traceable. Same amendment will forward-cover the read-time signatures path discussed for task 0122.

## Issues Encountered

- **Phantom-duplicate detection lag**: ~2 weeks elapsed between sibling task completion (0150/0046) and recognition that this task was a duplicate. Preventive guidance for future sessions: when activating a task whose ACs overlap with recently-archived sibling, run a duplicate-check pass before committing to scope.

## Future Work

Spawned as backlog task **0186** — ADR 0004 amendment documenting read-time XDR decode for S3-fetched archive payloads. The amendment will retrospectively cover ADRs 0033/0034 and forward-cover the read-time decode signatures path under consideration for task 0122.
