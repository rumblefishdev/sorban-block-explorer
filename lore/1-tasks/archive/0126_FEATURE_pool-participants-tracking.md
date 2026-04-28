---
id: '0126'
title: 'LP: pool participants and share tracking'
type: FEATURE
status: completed
related_adr: ['0008', '0024', '0037']
related_tasks: ['0043', '0052', '0077', '0136', '0162']
tags: [priority-low, effort-medium, layer-api, layer-db, audit-gap]
milestone: 1
links:
  - crates/api/src/liquidity_pools/mod.rs
  - crates/api/src/liquidity_pools/dto.rs
  - crates/api/src/liquidity_pools/handlers.rs
  - crates/api/src/liquidity_pools/queries.rs
  - crates/api/src/main.rs
  - crates/api/src/tests_integration.rs
  - docs/audits/2026-04-10-pipeline-data-audit.md
history:
  - date: '2026-04-10'
    status: backlog
    who: stkrolikiewicz
    note: 'Spawned from pipeline audit — tech design specifies pool participants table on LP detail page but no schema exists.'
  - date: '2026-04-24'
    status: blocked
    who: stkrolikiewicz
    by: ['0162']
    note: >
      Real blocker retargeted. 0136 was listed as blocker but is
      superseded; the actual prerequisite is parser work emitting
      pool_share trustlines as ExtractedLpPosition rows (today
      skipped at xdr-parser/src/state.rs:231-234). Spawned 0162 to
      cover that parser gap. Once 0162 lands, 0126 owns persist +
      API for per-provider share tracking.
  - date: '2026-04-24'
    status: blocked
    who: stkrolikiewicz
    note: >
      Scope reconciled with existing schema. `lp_positions` table
      from migration 0006 §16 already carries the exact shape tech
      design requires (`pool_id`, `account_id`, `shares`,
      `first_deposit_ledger`, `last_updated_ledger`). Earlier README
      draft proposed a new `liquidity_pool_participants` table —
      that was redundant. No DDL change needed; task is now
      persist + API only.
  - date: '2026-04-27'
    status: active
    who: stkrolikiewicz
    note: >
      Unblocked. 0162 merged on develop (PR #129) — parser now emits
      `ExtractedLpPosition` from pool_share trustline changes and
      persist already lands rows in `lp_positions` (verified by
      `synthetic_ledger_insert_and_replay_is_idempotent` regression).
      Remaining 0126 scope: API endpoint(s) for pool participants
      list + per-account LP holdings, frontend integration, and the
      prune-vs-keep decision for zero-share rows from
      `removed`-typed trustline changes.
  - date: '2026-04-28'
    status: completed
    who: stkrolikiewicz
    note: >
      Shipped on `feat/0126_pool-participants-tracking`. New
      `crates/api/src/liquidity_pools/` module exposes
      `GET /v1/liquidity-pools/{pool_id}/participants` —
      cursor-paginated, ordered by `shares DESC, account_id DESC`,
      filters `shares > 0`, returns 404 on missing pool. Built on the
      shared `common::*` primitives from task 0043 (PR #124):
      `Pagination<SharesCursor>` extractor, `cursor::encode/decode`,
      `errors::*` envelope, `pagination::finalize_page +
      into_envelope`. 5 new integration tests in
      `tests_integration.rs` (3 validation, 1 missing-pool 404, 1 e2e
      with seeded fixture covering sort, pagination round-trip,
      zero-share filter). Persist path was already in place from
      task 0149 + 0162 — no write.rs changes. Zero DDL.
---

# LP: pool participants and share tracking

## Summary

Tech design spec [§224 of `technical-design-general-overview.md`,
§492 of `frontend-overview.md`] required a "Pool participants" section
on the LP detail page listing providers and their share. Schema
(`lp_positions`) and parser (after 0162) and persist (after 0149) were
already in place; this task added the **API endpoint** that surfaces
the data.

`GET /v1/liquidity-pools/{pool_id}/participants` returns a
cursor-paginated list of `(account StrKey, shares, first_deposit_ledger,
last_updated_ledger)` ordered by `shares DESC`, leveraging the
pre-existing partial index `idx_lpp_shares (pool_id, shares DESC) WHERE
shares > 0`.

## Implementation

| File                                         | Δ                                                |
| -------------------------------------------- | ------------------------------------------------ |
| `crates/api/src/liquidity_pools/mod.rs`      | +20 (new — router)                               |
| `crates/api/src/liquidity_pools/dto.rs`      | +33 (new — `SharesCursor`, `ParticipantItem`)    |
| `crates/api/src/liquidity_pools/handlers.rs` | +95 (new — `list_participants`)                  |
| `crates/api/src/liquidity_pools/queries.rs`  | +83 (new — `pool_exists`, `fetch_participants`)  |
| `crates/api/src/main.rs`                     | +2 (mod decl + router mount)                     |
| `crates/api/src/tests_integration.rs`        | +220 (5 new integration tests + fixture helpers) |

**Tests**: 67/67 api crate (was 62 + 5 new for participants). Clippy
`--workspace --all-targets -- -D warnings` clean.

**Migrations**: none.

## Acceptance Criteria

- [x] `upsert_lp_positions` wired into persist path (watermark-guarded,
      `first_deposit_ledger` preserved on update). **Pre-existing** —
      shipped by task 0149 wiring; verified by 0162 integration test.
- [x] Withdrawal / zero-share handling documented and tested. Chose
      **option A**: zero-share rows persist (parser emits them on
      `removed` per 0162's emerged decision #2); API filters
      `WHERE shares > 0` in the SQL. Tested in
      `lp_participants_e2e_sort_filter_pagination` — fixture seeds a
      zero-share row, asserts it never appears in any page of the
      response. Rationale in Design Decisions §1 below.
- [x] `GET /liquidity-pools/{pool_id}/participants` returns
      per-provider shares, sorted by share size, cursor paginated.
      Returns 404 on missing pool, 400 on malformed pool_id / limit /
      cursor with the canonical `ErrorEnvelope` shape.
- [x] `lp_positions` populated end-to-end from `pool_share` trustlines
      after 0162 lands. **Pre-existing** — verified by
      `synthetic_ledger_insert_and_replay_is_idempotent` (in indexer
      crate) plus `lp_participants_e2e_sort_filter_pagination` (in api
      crate, hits the live DB through the real router).
- [x] Does NOT introduce `liquidity_pool_participants` — reconciled
      to existing `lp_positions`.
- [x] Zero DDL change required — confirmed.

## Design Decisions

### From Plan

1. **Zero-share rows kept in DB; API filters `WHERE shares > 0`.**
   Picked over the alternative (DELETE on zero in persist) for several
   reasons: (a) DB cost negligible — at mainnet withdrawal rates the
   zero-row accumulation is bounded at ~10–15 MB even after 5 years,
   <0.001% of overall schema size; (b) the partial UNIQUE index
   `idx_lpp_shares … WHERE shares > 0` is already designed for exactly
   this pattern (zero rows aren't in the index, so query performance is
   unaffected); (c) keeping zero rows preserves the data needed for
   future "former LPs" / time-series LP analytics features without an
   expensive re-index from operations history; (d) consistent with
   `account_balances_current` (also a soft-state accumulator that
   doesn't prune zero balances). Counter — option B gives "table =
   active LPs" semantic — was rejected because the cleaner mental
   model is not worth losing future-proofing.

2. **Custom `SharesCursor` payload (not `TsIdCursor`).** The natural
   ordering for participants is `(shares DESC, account_id DESC)`, not
   `(created_at, id)`. Generic `cursor::{encode, decode}` from PR #124
   handles arbitrary `Serialize + DeserializeOwned` payloads, so a
   bespoke struct fits without new helpers.

3. **`account_id_surrogate` carried internally for cursor tie-breaker;
   stripped from response.** SQL predicate `(lpp.shares, lpp.account_id) <
($cur_shares, $cur_acct_id)` needs the BIGINT surrogate, but the
   public DTO carries only the StrKey. Internal `ParticipantRow`
   carries both; `ParticipantItem` (response DTO) drops the surrogate.
   Cursor stays opaque per ADR 0008.

### Emerged

4. **404 for missing pool, not 200-with-empty-list.** Matches
   `contracts::list_invocations` / `list_events` convention — listing a
   non-existent parent resource is an error, not an empty success.
   Costs one extra `pool_exists` lookup per request; cheap (PK lookup
   on `liquidity_pools.pool_id`), worth it for honest semantics.

5. **Pool ID validated as 64-char lowercase hex inline; not via
   `common::filters::strkey`.** Strkey helper is for `G…` / `C…`
   accounts; pool_id is a 32-byte BYTEA hash rendered as hex. Inline
   `is_valid_pool_id_hex` in handlers.rs — could be promoted to
   `common::filters::pool_id_hex` once a second consumer appears.

## Issues Encountered

None.

## Future Work

- **Frontend integration** — pool detail page renders the participants
  list. Separate task; out of 0126 scope.
- **Per-account LP holdings endpoint** —
  `GET /v1/accounts/{strkey}/liquidity-positions` would invert the
  index and let users see "what pools is this account in?". Different
  query (different index path), different module placement
  (`accounts/`). Not a 0126 follow-up; spawn when account-detail
  endpoint module starts.
- **`common::filters::pool_id_hex`** — promote inline validator if a
  second handler ever needs it.

## Notes

Closes the persist + API path for the third 2026-04-10 audit-gap
(0162 was the parser, 0126 is the surface). After 0126 ships, every
mainnet `pool_share` trustline change flows through end-to-end:
parser → persist → DB → `/v1/liquidity-pools/{id}/participants` →
frontend. Feature parity with Stellar Expert
(`/explorer/public/liquidity-pool/{id}/holders`) per
2026-04-27 verification probe.
