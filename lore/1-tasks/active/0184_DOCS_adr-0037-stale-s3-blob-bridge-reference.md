---
id: '0184'
title: 'DOCS: drop ADR 0037 stale `parsed_ledger_{N}.json` bridge reference (superseded by ADR 0029)'
type: DOCS
status: active
related_adr: ['0029', '0032', '0037']
related_tasks: ['0047', '0175']
tags: [docs, adr-drift, evergreen, effort-small, priority-low]
links:
  - lore/2-adrs/0037_current-schema-snapshot.md
  - lore/2-adrs/0029_read-time-xdr-fetch.md
  - lore/2-adrs/0032_docs-architecture-evergreen-maintenance.md
history:
  - date: '2026-04-30'
    status: active
    who: stkrolikiewicz
    note: >
      Surfaced during E05 manual endpoint audit (task 0175): ADR 0037
      §"Identifier types" line 119 still claims `ledger_sequence` is
      a "bridge column to S3 `parsed_ledger_{N}.json`", but ADR 0029
      abandoned the parsed-ledger artifact storage track in favour of
      read-time XDR fetch. The S3 blob plan is gone (PR #139, lore-0047
      docs/architecture/database-schema/endpoint-queries/05_get_ledgers_by_sequence.sql
      carries the SUPERSESSION NOTE 2026-04 documenting this). The
      `ledger_sequence` column remains for lookup ergonomics, but the
      rationale recorded in ADR 0037 is dead and misleads future
      readers about the storage topology. Per ADR 0032 evergreen rule,
      ADR drift on the schema-shape surface should be patched in-place
      with a forward link to the superseding ADR.
---

# Drop ADR 0037 stale `parsed_ledger_{N}.json` bridge reference

## Summary

`lore/2-adrs/0037_current-schema-snapshot.md` line 119 references a
storage artifact (`parsed_ledger_{N}.json` per-ledger S3 blob) that
ADR 0029 abandoned. The reference is purely descriptive — no code or
schema depends on it — but it leaves future readers with a wrong mental
model of the storage topology and contradicts the canonical SQL spec
([`05_get_ledgers_by_sequence.sql`](../../../docs/architecture/database-schema/endpoint-queries/05_get_ledgers_by_sequence.sql)
SUPERSESSION NOTE 2026-04).

## Context

ADR 0037 was sealed as a frozen snapshot of the schema at migration
`20260424000000`. Per [ADR 0039](../../2-adrs/0039_delta-adr-pattern.md)
(if it exists, otherwise the convention) thin follow-up ADRs append
deltas without rewriting 0037 itself. The "bridge to parsed_ledger blob"
line is not a schema fact — it's a *rationale* attached to a column —
and has been falsified by ADR 0029. Updating the rationale in-place is
preferable to a delta-ADR for a non-shape correction.

## Acceptance Criteria

- [x] ADR 0037 line 119 rewritten to drop the `parsed_ledger_{N}.json`
      claim and forward-link to ADR 0029 for the read-time XDR fetch
      replacement
- [x] All other ADR 0037 stale "S3 bridge" rationale lines patched
      (lines 587 ERD intro, 799 `ledgers` no-FK rationale)
- [x] `docs/architecture/database-schema/endpoint-queries/README.md`
      data-source matrix and E5 section rewritten to reflect the
      DB-only (post-ADR-0029) replacement; the supersession note
      mirrors the wording in `05_get_ledgers_by_sequence.sql`
- [x] No `parsed_ledger_{N}.json` references remain in
      `lore/2-adrs/0037_*.md` or `docs/architecture/**` outside of
      explicit supersession notes that name the abandoned artifact for
      traceability (older ADRs 0011/0013/0016/0020/0025 are
      historical records, intentionally untouched)
- [x] PR description records this as an evergreen-doc patch under
      ADR 0032

## Notes

- **Discovered alongside E05 audit.** Karol's PR #139 (lore-0047)
  followed the SUPERSESSION NOTE; the audit caught that ADR 0037's
  internal description never got the same update. Cleanup-only.
- **Out of scope:** any actual schema change to `ledger_sequence`
  semantics. Column stays as-is; only the rationale in ADR 0037 is
  corrected.
