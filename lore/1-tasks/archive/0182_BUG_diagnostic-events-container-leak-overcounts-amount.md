---
id: '0182'
title: 'BUG: diagnostic_events container leak — Contract-typed mirrors overcount soroban_events_appearances.amount ~2-3x'
type: BUG
status: completed
related_adr: ['0033']
related_tasks: ['0173']
tags: ['xdr-parser', 'staging', 'data-correctness', 'effort-small', 'cap-67']
links: []
history:
  - date: '2026-04-29'
    status: backlog
    who: fmazur
    note: >
      Spawned from 0173 cross-validation against stellar.expert.
      Comparison of per-tx event counts (Soroswap swap on ledger 62016099)
      revealed our `amount` is 2.5× higher than stellar.expert's canonical
      consensus-event count. Root cause: staging filter is type-based
      (drops `event_type == Diagnostic`), but Stellar core mirrors every
      consensus Contract event into `v4.diagnostic_events` with inner
      `type_ = Contract`, which slips through the filter as a duplicate.
  - date: '2026-04-29'
    status: active
    who: fmazur
    note: Promoted from backlog to active.
  - date: '2026-04-29'
    status: completed
    who: fmazur
    note: >
      Implementation complete. Added `EventSource` enum (TxLevel/PerOp/Diagnostic)
      + `source` field on `ExtractedEvent`; parser tags every event at the
      extraction site for V3 + V4 (× 3 containers). Staging, read-time API
      (split_events, /contracts/:id/events), and NFT detection now filter
      on `source == Diagnostic` instead of inner `event_type`. 4 new unit
      tests + 1 new integration test (v4_diag_contract_mirror_does_not_inflate_amount).
      Empirical scan over 394 342 V4 txs (1000 ledgers) confirmed: 100%
      mirror rate for Soroban txs; 0 orphan Contract events from successful
      txs (3 644 orphans all from FAILED txs — pre-fix bug, drop is correct).
      Backfill of 100-ledger sample produced canonical counts: swap tx
      1c61a3b7…2438 → CA6PUJLB=2, CAS3J7GY=3, CCW67TSZ=1 (total 6, was 10).
      Cross-validated against live stellar-rpc (transactionEventsXdr +
      contractEventsXdr structure matches our tx-level + per-op indexing).
---

# BUG: diagnostic_events container leak — Contract-typed mirrors overcount soroban_events_appearances.amount ~2-3×

## Summary

`crates/indexer/src/handler/persist/staging.rs:680-683` filters events by
the **inner** `event.type_` (drops `Diagnostic`), not by the **container**
they came from. Stellar core's V4 meta mirrors every consensus Contract
event from `v4.operations[i].events` into `v4.diagnostic_events` with
the same inner `type_ = Contract` (byte-identical). The filter passes
both copies through, so `soroban_events_appearances.amount` counts each
consensus event **twice**.

Empirical: on the Soroswap swap tx
`1c61a3b7…2438` (ledger 62016099), DB shows `amount` summing to 10
across the 3 contracts; stellar.expert renders 4 distinct events for
the same tx (the canonical consensus set from
`v4.operations[0].events`). Our index over-reports by ~2.5×. This is
~50% noise on the appearance index pagination and breaks alignment
with every other Stellar tool.

## Context

ADR 0033 says the index "only counts contract and system events" and
diagnostic content lives in the S3/archive read lane. The intent was
correct; the implementation matched the wrong signal:

- **Hashed (consensus)**: `v4.events` (tx-level fee/refund) +
  `v4.operations[i].events` (per-op Soroban + classic SAC unification).
  These are part of `txSetResultHash`.
- **NOT hashed**: `v4.diagnostic_events`. CAP-67 spec: "diagnostic
  events are auxiliary debug information not hashed into the ledger."

What's surprising: `v4.diagnostic_events` carries entries with
`type_ ∈ {Contract, Diagnostic}`. The `Contract`-typed entries are a
**byte-identical mirror** of the consensus per-op events (verified
empirically — every per-op event on the Soroswap swap tx appears as a
matching Contract-typed entry in `diagnostic_events`, full topics+data
equality). Stellar core emits this duplicate when
`--diagnostic-events` is enabled (default for archive-bound captive
core). The Diagnostic-typed entries (host VM trace: `core_metrics`,
`fn_call`, `fn_return`, error info) get dropped correctly today.

### Concrete numbers from local 100-ledger sample (post-task-0173)

| Source                                | Ledger 62016099 swap tx (`1c61a3b7…`)          |
| ------------------------------------- | ---------------------------------------------- |
| `v4.events` (tx-level)                | 2 (XLM SAC fee × 2)                            |
| `v4.operations[0].events`             | 4 (CAS3J7GY×1 + CCW67TSZ×1 + CA6PUJLB×2)       |
| `v4.diagnostic_events` Contract-typed | 4 (BYTE-IDENTICAL mirror of per-op)            |
| `v4.diagnostic_events` Diagnostic     | ~20 (filtered correctly)                       |
| **Current DB amount sum**             | **10** (2 tx-level + 4 per-op + 4 diag-mirror) |
| **Stellar.expert renders**            | **4** (per-op only)                            |

Globally on 100-ledger sample: post-task-0173 SUM(amount) = 92,335.
Estimated post-fix: ~46K (a 2× reduction). Aligns with stellar.expert's
canonical view.

## Implementation Plan

### Step 1 — Tag `ExtractedEvent` with its source container

`crates/xdr-parser/src/types.rs` — add:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventSource {
    TxLevel,    // v4.events / v3 soroban_meta.events
    PerOp,      // v4.operations[i].events (CAP-67)
    Diagnostic, // v4.diagnostic_events / v3 soroban_meta.diagnostic_events
}

pub struct ExtractedEvent {
    // ...existing fields...
    pub source: EventSource,
}
```

### Step 2 — Parser tags at extraction site

`crates/xdr-parser/src/event.rs::extract_events` — populate `source`
on each `extract_single_event` call so the V3/V4 dispatch records
where the event came from. V3: `meta.events` → TxLevel,
`meta.diagnostic_events` → Diagnostic. V4: `v4.events` → TxLevel,
`v4.operations[i].events` → PerOp, `v4.diagnostic_events` → Diagnostic.

Read-time API (`crates/api/src/stellar_archive/extractors.rs::split_events`,
`crates/api/src/contracts/handlers.rs`) is unaffected — it can still
expose Diagnostic events for E03 advanced mode if it wants; just route
on `source` instead of `event_type`.

### Step 3 — Staging filters by source, not by inner type

`crates/indexer/src/handler/persist/staging.rs:680-683`:

```rust
// Replace
if ev.event_type == ContractEventType::Diagnostic { continue; }
// with
if ev.source == EventSource::Diagnostic { continue; }
```

This drops the entire diagnostic_events container, including its
Contract-typed mirrors, and keeps tx-level + per-op consensus events.

### Step 4 — Tests

Unit tests in `event.rs::tests`:

1. **V4 source tagging** — build V4 meta with one event in each of the
   three locations + a Contract-typed entry in `diagnostic_events`;
   assert `source` is `TxLevel / PerOp / Diagnostic / Diagnostic`
   respectively.
2. **V3 source tagging** — build V3 meta; assert
   `meta.events → TxLevel` and `meta.diagnostic_events → Diagnostic`.

Persist integration test in `persist_integration.rs`:

3. **Diag-Contract mirror is dropped** — extend or fork the existing
   `v4_per_op_events_land_in_appearance_index` test:
   build V4 meta with 1 per-op Contract event + 1 byte-identical
   Contract-typed entry in `diagnostic_events`; run extract_events →
   persist_ledger; assert `soroban_events_appearances.amount = 1`,
   not 2. Pre-fix this would have produced 2.

### Step 5 — Reindex consideration

Existing rows in DB have inflated `amount`. Re-ingest of the local
100-ledger sample is a 30 s job. Owner's call whether to reindex or
just let it stand for the existing local data — fix is forward-only.

## Acceptance Criteria

- [x] `ExtractedEvent.source: EventSource` field added; parser populates
      it on every extraction site (V3 + V4 × 3 containers)
- [x] Staging filter switched from `event_type == Diagnostic` to
      `source == Diagnostic`; semantically drops the entire
      `*.diagnostic_events` vec including Contract-typed mirrors
- [x] Read-time API call sites updated to filter on `source` (went a step
      further than "unaffected" — split_events + /contracts/:id/events
      both routed on EventSource::Diagnostic to avoid the same mirror-leak
      at read time)
- [x] Unit tests cover V3 + V4 source tagging (4 new tests in
      `event.rs::tests`)
- [x] Integration test asserts diag-Contract mirror does NOT increment
      `amount` (`v4_diag_contract_mirror_does_not_inflate_amount` in
      persist_integration.rs)
- [x] Sample re-ingest: `amount` for Soroswap swap tx
      (`1c61a3b7b21ab48c6f02b72d124b4da86196091558d00c2879969d29a5ce2438`,
      ledger 62016099) drops to consensus-only counts:
      CCW67TSZ=1, CA6PUJLB=2, CAS3J7GY=3 (was 2/4/4) — confirmed in DB
      after backfill of 100-ledger sample.
- [x] **Docs updated** — `docs/architecture/xdr-parsing/xdr-parsing-overview.md`
      §5.1 V3↔V4 dispatch section: clarified that diagnostic_events
      container is dropped at staging regardless of inner type, and
      that Stellar core mirrors consensus Contract events into the
      diagnostic container (byte-identical duplicate).
      `N/A — schema unchanged` for ADR 0033/0037 (only filter shifts;
      table shape stays).

## Implementation Notes

Files touched (8 production + 1 example + 1 test + 1 doc):

- `crates/xdr-parser/src/types.rs` — added `EventSource` enum + `source`
  field on `ExtractedEvent`.
- `crates/xdr-parser/src/event.rs` — `extract_events` tags every event
  with its source container; 4 new unit tests for V3/V4 tagging including
  the byte-identical-mirror reproduction test.
- `crates/xdr-parser/src/lib.rs` — re-exports `EventSource`.
- `crates/xdr-parser/src/nft.rs` — NFT detection skips diagnostic-source
  events (would otherwise double-emit transfer/mint/burn from mirrors).
- `crates/indexer/src/handler/persist/staging.rs` — filter switched from
  `event_type == Diagnostic` to `source == EventSource::Diagnostic`;
  comment updated with task-0182 rationale.
- `crates/api/src/stellar_archive/extractors.rs` — `split_events` routes
  on source.
- `crates/api/src/contracts/handlers.rs` — `/contracts/:id/events`
  filters on source.
- `crates/indexer/tests/persist_integration.rs` — 1 new integration test
  - `make_transfer_event` updated for new field.
- `docs/architecture/xdr-parsing/xdr-parsing-overview.md` — new §5.1
  subsection "Source-container tagging (task 0182)" + table mapping
  source location → EventSource → counts-in-amount.
- `crates/xdr-parser/examples/diag_overlap_check.rs` — new diagnostic
  tool used during implementation; kept as long-term verification asset
  (run on any partition dir to confirm orphan invariants for future
  Galexie / stellar-core upgrades).

Test counts: xdr-parser **179 unit tests** (+4 new), indexer
`persist_integration` **11 tests** (+1 new), api **113 tests** — all
passing.

## Design Decisions

### From Plan

1. **`EventSource` enum, not `Option<bool>`/string flag**: a typed enum
   gives the compiler exhaustive matching (every parser path must pick
   one) and makes downstream filters self-documenting
   (`source == EventSource::Diagnostic` reads obviously).
2. **Filter at staging, not at parser**: parser still emits all events
   tagged with their source. Staging is the only consumer that drops by
   source. Read-time API gets the full set and chooses what to surface
   per endpoint.

### Emerged

3. **Updated read-time API too** (split_events, /contracts/:id/events,
   NFT detect). Original task plan said these were "unaffected — can
   route on source if needed". Empirically the same bug existed there:
   read-time handlers used `event_type == Diagnostic` filter, which let
   the byte-identical Contract-typed mirrors leak through. Switching
   them to `source == Diagnostic` was a one-line change per site and
   keeps behavior consistent across ingest and read paths.
4. **NFT detection guard**: `detect_nft_events` already filtered on
   `event_type != Contract`, but the V4 mirror keeps `type_ = Contract`,
   so NFT detection would have double-emitted transfer/mint/burn for
   every soroban tx. Added an explicit `source == Diagnostic` skip.
5. **Kept `diag_overlap_check.rs` as an example** instead of throwing
   it away. The 394 342-tx empirical study it ran was the only thing
   that surfaced the orphan-from-failed-tx pattern (Contract events
   in diag without a per-op match — all from failed contract calls).
   Future Galexie or stellar-host changes might reintroduce a leak;
   this tool re-runs in seconds on any partition dir and pins the
   invariant "0 orphans from successful tx".

## Issues Encountered

- **Initial root-cause hypothesis was incomplete.** Task description
  said stellar-core mirrors per-op events into diagnostic_events
  byte-identically. Two delegated subagents traced stellar-core C++
  source and concluded no mirror exists. Empirical scan on user's
  100-ledger sample then confirmed the mirror IS happening — but it's
  produced by **soroban-host (Rust)**, not by stellar-core (C++).
  The host emits each contract event into BOTH `output.contract_events`
  (→ per-op) AND `output.diagnostic_events` (→ diag) when diagnostic
  mode is on. Stellar-core just routes them to different containers
  without any copy. The SDK comment "diagnosticEvents MAY include
  contract events as well" is the canonical Stellar-side acknowledgment.
  Code comments and docs were written to be accurate without claiming a
  specific source-of-duplication mechanism (just: "the container can
  contain Contract-typed entries, drop the whole container by source").
- **Orphan Contract events in diag from FAILED txs.** 3 644 orphans
  found in 1000-ledger scan (≈0.9% of V4 txs with diag). Every single
  one came from a failed transaction (TxFailed / TxFeeBumpInnerFailed)
  with `inSuccessfulContractCall=false`. These are events soroban-host
  emitted before the call rolled back; not consensus, not in the per-op
  vec, not indexed by stellar-rpc/horizon/stellar.expert. Pre-fix our
  staging counted them as real events on failed tx — that was a latent
  bug, fix corrects it.
- **Cross-validation against stellar.expert via WebFetch failed**
  because their UI is a client-rendered SPA. Worked around by decoding
  raw XDR meta directly (own parser tool) — same data the validators
  produced, authoritative.
- **stellar-rpc retention is ~7 days**, so the swap tx (4 weeks old)
  could not be queried live. Cross-validated by querying a fresh tx in
  retention window: confirmed `getTransaction.events` exposes
  `transactionEventsXdr` (= our tx-level) + `contractEventsXdr` (= our
  per-op), with `diagnosticEventsXdr` separate (dropped from
  `getEvents` index). Same matheamatics as our fix.

## Notes

- This is a **task-0173 follow-up**: 0173 added per-op events to the
  ingest path; this task removes the redundant diag-container mirror
  that was always being counted. Together they bring the appearance
  index to consensus-only correctness.
- **Tx-level fee events** (`v4.events` — fee charge + refund on XLM
  SAC) are still kept in this fix — they're consensus and represent
  real "contract was touched" appearances. Stellar.expert filters
  them from per-contract event lists at the UI layer (presentation
  choice, not consensus filter); leaving them in the DB index keeps
  the count complete and lets the API decide per-endpoint.
- Out of scope:
  - Reindex of the local sample (owner's call — forward-only fix).
  - Whether E14 `/contracts/:id/events` should hide tx-level fee
    events à la stellar.expert (separate UX task if requested).
  - Mainnet operational concerns: no production DB yet, so no
    migration plan needed.
