---
id: '0166'
title: 'API: CrudResource trait + crud_routes! macro — extract when first simple consumer lands'
type: FEATURE
status: backlog
related_adr: ['0008']
related_tasks: ['0043', '0047']
tags: [layer-backend, abstraction, effort-small, priority-low, rule-of-three]
links: []
history:
  - date: '2026-04-24'
    status: backlog
    who: karolkow
    note: >
      Spawned from 0043 post-implementation audit. Initial 0043 scope (per
      stkrolikiewicz 2026-03-30 sync) included a CrudResource trait +
      crud_routes! macro. The trait + macro were implemented and landed on
      feat/0043_backend-pagination-query-parsing. A subsequent roadmap audit
      against 0045-0053 endpoint specs showed 0/13 planned list endpoints
      match the trait's hardcoded shape (TsIdCursor-only ordering, zero
      filter params, no enrichment hook). stkrolikiewicz agreed to defer.
      Trait + macro removed from 0043 PR; this task tracks the re-add when
      a simple consumer actually lands.
---

# API: CrudResource trait + crud_routes! macro — extract when first simple consumer lands

## Summary

Re-extract the list/detail handler scaffolding (CrudResource trait + crud_routes! macro) once a real API resource exists that genuinely fits the "simple cursor + no filters + no enrichment" shape. Until then the low-level helpers in `crates/api/src/common/{cursor,pagination,filters,extractors,errors}.rs` are sufficient and every endpoint composes them directly.

## Context

Task 0043 originally shipped a `CrudResource` trait and `crud_routes!` macro alongside the shared pagination / filter / error helpers. Post-implementation audit against the planned roadmap (0045-0053 backend modules) showed the trait's shape does not match any of the 13 planned list endpoints:

| Endpoint                             | Trait mismatch                                                     |
| ------------------------------------ | ------------------------------------------------------------------ |
| `/transactions` (0046)               | S3 memo enrichment does not fit `into_item(row) -> item`           |
| `/ledgers` (0047)                    | Uses `sequence` cursor, not `(created_at, id)`                     |
| `/ledgers/{seq}/transactions`        | Nested route (not top-level resource)                              |
| `/accounts/{id}/transactions` (0048) | Nested route                                                       |
| `/assets` (0049)                     | Requires `filter[type]`, `filter[code]` — trait has no filter slot |
| `/assets/{id}/transactions`          | Nested route + join-dependent query                                |
| `/contracts/*` (0050)                | Multi-endpoint (interfaces, invocations, events)                   |
| `/nfts` (0051)                       | Requires `filter[collection]`, `filter[contract_id]`               |
| `/nfts/{id}/transfers`               | Derived from events, not a table                                   |
| `/liquidity-pools` (0052)            | Requires `filter[assets]`, `filter[min_tvl]`                       |
| `/liquidity-pools/{id}/transactions` | Nested route                                                       |
| `/search` (0053)                     | Cross-entity custom query                                          |

Rule of three: we had one theoretical candidate (ledgers-without-filters) — not three. Premature abstraction was shipped and removed in the same 0043 PR per agreement with stkrolikiewicz.

The helpers in `common/*` remain — they are generic (`cursor::encode/decode<P>`, `finalize_page<Row>` with caller-provided cursor fn, `Pagination<P>` generic over payload type) and adopt cleanly across all 13 endpoints.

## Trigger Condition

Re-open this task when **two** of the following land without filter/enrichment requirements that force them to bypass the trait:

- 0047 `/ledgers` list (with `SequenceCursor { seq }` payload)
- A future simple resource (e.g. a read-only metadata table)

At that point the trait and macro can be re-extracted — **rewritten to accept a generic `Cursor` type** (not hardcoded to `TsIdCursor`) — and both endpoints retrofitted onto it.

## Implementation Plan

1. Confirm trigger condition (two simple-shape list endpoints exist).
2. Recover the original trait + macro skeleton from `.trash/api_common_crud.rs` in git history (`git log --all --oneline -- .trash/api_common_crud.rs` — final revision at commit of archival).
3. Rewrite `CrudResource` with a `type Cursor: Serialize + DeserializeOwned` associated type instead of hardcoding `TsIdCursor`.
4. Rewrite `crud_routes!` to accept the cursor type (likely via an extra macro parameter or via `<R as CrudResource>::Cursor`).
5. Retrofit both trigger-condition endpoints onto the trait.
6. Delete any duplicated handler bodies from the retrofitted resources.

## Acceptance Criteria

- [ ] Two real resources implement `CrudResource` and register via `crud_routes!`.
- [ ] Trait is generic over cursor payload (not `TsIdCursor`-only).
- [ ] No warnings / allow-lints required for the trait or macro on a `-D warnings` build.
- [ ] No existing handler regresses (wire contract unchanged for retrofitted resources).
- [ ] **Docs updated** — N/A expected (internal refactor; no wire shape change).

## Notes

- Original implementation + tests live in `.trash/api_common_crud.rs` at the 0043 archival commit. Treat it as a starting point, not a blueprint — the cursor generalisation is load-bearing and requires macro changes.
- If after 0047 + one more simple resource the trait still does not obviously help, close this task as `canceled` (reason: `obsolete`) rather than forcing it in.
