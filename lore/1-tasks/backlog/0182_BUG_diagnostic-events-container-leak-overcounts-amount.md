---
id: '0182'
title: 'BUG: diagnostic_events container leak — Contract-typed mirrors overcount soroban_events_appearances.amount ~2-3x'
type: BUG
status: backlog
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

- [ ] `ExtractedEvent.source: EventSource` field added; parser populates
      it on every extraction site (V3 + V4 × 3 containers)
- [ ] Staging filter switched from `event_type == Diagnostic` to
      `source == Diagnostic`; semantically drops the entire
      `*.diagnostic_events` vec including Contract-typed mirrors
- [ ] Read-time API call sites unaffected (still receive all events;
      can route on `source` if needed)
- [ ] Unit tests cover V3 + V4 source tagging
- [ ] Integration test asserts diag-Contract mirror does NOT increment
      `amount`
- [ ] Sample re-ingest: `amount` for Soroswap swap tx
      (`1c61a3b7b21ab48c6f02b72d124b4da86196091558d00c2879969d29a5ce2438`,
      ledger 62016099) drops to consensus-only counts:
      CCW67TSZ=1, CA6PUJLB=2, CAS3J7GY=3 (was 2/4/4)
- [ ] **Docs updated** — `docs/architecture/xdr-parsing/xdr-parsing-overview.md`
      §5.1 V3↔V4 dispatch section: clarify that diagnostic_events
      container is dropped at staging regardless of inner type, and
      that Stellar core mirrors consensus Contract events into the
      diagnostic container (byte-identical duplicate).
      `N/A — schema unchanged` for ADR 0033/0037 (only filter shifts;
      table shape stays).

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
