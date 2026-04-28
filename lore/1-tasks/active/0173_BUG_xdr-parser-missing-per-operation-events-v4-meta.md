---
id: '0173'
title: 'BUG: xdr-parser drops per-operation events in V4 meta (CAP-67 / Protocol 23+)'
type: BUG
status: active
related_adr: ['0002', '0033']
related_tasks: ['0167', '0026']
tags:
  [
    bug,
    indexer,
    xdr-parser,
    cap-67,
    protocol-23,
    events,
    priority-critical,
    pre-mainnet-backfill,
  ]
links:
  - 'crates/xdr-parser/src/event.rs'
  - 'lore/2-adrs/0002_rust-ledger-processor-lambda.md'
history:
  - date: 2026-04-28
    status: backlog
    who: fmazur
    note: >
      Spawned from manual E02 verification of contract_ids[] field.
      Discovered while comparing tx 358ef42d (KALE harvest) vs Horizon:
      KALE SAC mint event missing from soroban_events_appearances
      despite Horizon asset_balance_changes confirming a 0.328 KALE
      mint to CBHQMW7M756644YXWBLT4FBO5X4EQJ5IEWLLQU4DFAHEABPDLCTP66ZW.
      Root cause traced to crates/xdr-parser/src/event.rs:50-76 — the V4
      branch reads v4.events (tx-level) and v4.diagnostic_events but
      does NOT iterate v4.operations[i].events where Protocol 23+ places
      per-operation contract events. ADR 0002 explicitly requires the
      latter; SorobanTransactionMetaV2 (V4 form) no longer carries the
      `events` field, so for Protocol 23+ ledgers ALL Soroban contract
      events emitted during InvokeHostFunction execution land in
      OperationMetaV2.events and are currently dropped.
  - date: 2026-04-28
    status: backlog
    who: fmazur
    note: >
      Bug reproduced deterministically with a synthetic control test:
      built a TransactionMeta::V4 with one event placed exclusively in
      operations[0].events (tx-level events empty, diagnostic events
      empty, soroban_meta=None). extract_events returned 0 events
      instead of the expected 1. This proves the bug is structural
      (not data-specific) and is in the parser code, not in the
      indexer pipeline or Stellar protocol.
  - date: 2026-04-28
    status: backlog
    who: fmazur
    note: >
      Cross-checked against upstream official sources to rule out any
      stale-doc / vendored-crate divergence. Both confirm:
      (1) CAP-67 (github.com/stellar/stellar-protocol/blob/master/core/cap-0067.md)
      states verbatim "Soroban events will be moved to OperationMetaV2".
      (2) Canonical XDR (github.com/stellar/stellar-xdr/blob/main/Stellar-ledger.x):
      OperationMetaV2 has `ContractEvent events<>`, TransactionMetaV4
      has `TransactionEvent events<>` (tx-level only),
      SorobanTransactionMetaV2 has only `ext` + `returnValue` (NO events
      field) — confirming events were structurally relocated, not
      duplicated. Local ADR 0002 quotes CAP-67 correctly; vendored
      stellar-xdr-26.0.0 crate reflects upstream definitions correctly;
      our extract_events alone is the gap.
  - date: 2026-04-28
    status: backlog
    who: fmazur
    note: >
      Quantified blast radius against the local 100-ledger sample
      (Protocol 25): of 13,308 Soroban tx, 11,082 (83.3%) carry
      exactly 2 events both emitted by native XLM SAC — i.e. only the
      Protocol 23 tx-level fee events, with zero execution events
      captured. 6,154 of 6,230 (98.8%) Soroban tx with any nested
      invocation have at least one "silent contract" — invoked in the
      call tree but absent from this tx's events. 21 of 55 (38%)
      distinct Soroban-callable contracts have zero event rows in the
      whole DB. Same gap also drops Protocol 23 unified events for the
      23,011 classic tx (per-op SAC transfer events on classic Payments
      / path payments). KALE was just the first concrete miss spotted.
      Bug is structural and uniformly drops 100% of `OperationMetaV2.events`
      regardless of contract / asset / op type.
  - date: 2026-04-28
    status: active
    who: fmazur
    note: 'Promoted to active via /promote-task'
---

# BUG: xdr-parser drops per-operation events in V4 meta (CAP-67 / Protocol 23+)

## Summary

`crates/xdr-parser/src/event.rs::extract_events` does not read
`TransactionMetaV4.operations[i].events`, the per-operation event location
introduced by CAP-67 / Protocol 23. ADR 0002 lists this as a required
parsing path. Empirically, mint/transfer/swap events emitted during
Soroban `InvokeHostFunction` execution and SAC events emitted by
classic operations are absent from `soroban_events_appearances` for
post-Protocol 23 ledgers, while tx-level fee events (in `v4.events`)
are present. Fix is local (one branch in one function) but blocks
data correctness ahead of mainnet backfill.

## Status: Backlog

**Current state:** root cause confirmed in code + ADR + XDR struct
definitions + empirical reproduction. Awaiting fix + reindex.

## Context

### What CAP-67 / Protocol 23 changed

`SorobanTransactionMeta` (V3, Protocol ≤ 22):

```rust
pub struct SorobanTransactionMeta {
    pub events: VecM<ContractEvent>,         // ALL Soroban events here
    pub return_value: ScVal,
    pub diagnostic_events: VecM<DiagnosticEvent>,
    ...
}
```

`SorobanTransactionMetaV2` (V4, Protocol ≥ 23):

```rust
pub struct SorobanTransactionMetaV2 {
    pub return_value: Option<ScVal>,         // events field REMOVED
    ...
}
```

Events were relocated to two new homes inside `TransactionMetaV4`:

```rust
pub struct TransactionMetaV4 {
    pub events: VecM<TransactionEvent>,       // tx-level (stage = BeforeAllTxs|AfterTx|AfterAllTxs)
    pub operations: VecM<OperationMetaV2>,    // each carries:
    //                                          per-op events (where SAC events land)
    pub diagnostic_events: VecM<DiagnosticEvent>,
    pub soroban_meta: Option<SorobanTransactionMetaV2>,  // no longer holds events
    ...
}

pub struct OperationMetaV2 {
    pub events: VecM<ContractEvent>,          // <-- Soroban + classic-via-SAC events
    ...
}
```

ADR 0002 §1: _"TransactionMetaV4 (introduced Protocol 23, CAP-0067;
active on mainnet Protocol 25) reorganizes events — fee events at
top-level, **per-operation events in `OperationMetaV2`**, soroban_meta
persists. The parser must dispatch on meta version (V3 vs V4)."_

### What our parser does

`crates/xdr-parser/src/event.rs::extract_events` line 49-76:

```rust
TransactionMeta::V4(v4) => {
    let mut extracted: Vec<ExtractedEvent> = v4
        .events                                    // tx-level only
        .iter()
        .enumerate()
        .map(|(i, tx_event)| { ... })
        .collect();
    // Include diagnostic_events
    let base = extracted.len();
    for (j, diag) in v4.diagnostic_events.iter().enumerate() {
        extracted.push(extract_single_event(&diag.event, ...));
    }
    extracted
    // ⚠ MISSING: for op_meta in v4.operations.iter() { for event in op_meta.events.iter() { ... } }
}
```

`v4.operations[i].events` is never read. For Protocol 23+ ledgers, this
is where most Soroban contract events live.

### Empirical reproduction

Local DB indexed against ledgers 62016000–62016099, all `protocol_version = 25`.

**Tx `358ef42d9840d91554a46d69be7c7fee8f8f4305379ab6ed614e4ea9ae4e75dc`** — Soroban
`harvest()` call on the KALE protocol:

- Horizon `asset_balance_changes`: `mint` of 0.328 KALE (classic asset,
  issuer `GBDVX4VEL...`) to recipient `CBHQMW7M756644YXWBLT...` (KALE SAC).
- Our `soroban_events_appearances` for this tx contains:
  `[CAS3J7GYLGX… (XLM SAC, fee event), CB23WRDQWGS… (unknown)]`
- **Missing:** `CBHQMW7M…` (KALE SAC, the mint event emitter).

**Tx `1c61a3b7…`** — Soroswap XLM↔USDC swap:

- Horizon `asset_balance_changes`: XLM in / USDC out via Soroswap router
  `CA6PUJLBYK…`.
- Our DB shows the router but **no swap-internal events** — those would
  live in `v4.operations[0].events` and are dropped.

### Why XLM SAC events DO appear (the partial coverage)

Classic Payment fees in Protocol 23+ emit a top-level `TransactionEvent`
(stage = `AfterTx`) via the native XLM SAC. Those land in `v4.events`
which we _do_ parse, hence `CAS3J7GYLGX…` (XLM SAC) shows up in
`soroban_events_appearances` for every classic XLM tx in our DB. This
is fee-event coverage only — not the unified Protocol 23 transfer/mint
event surface.

### Scope of impact (validated against local 100-ledger DB)

Hard numbers from local indexed sample (ledgers 62016000–62016099,
all Protocol 25). Bug is structural: drops 100% of per-op events on
V4 meta, regardless of contract / asset / op type. KALE was just the
first concrete case spotted — it is one of _many_.

| Metric                                                                                                                                                            | Value                                                                   |
| ----------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------- |
| Soroban tx total in DB                                                                                                                                            | 13,308                                                                  |
| Soroban tx with **exactly 2 events, both XLM SAC fee events** (zero execution coverage)                                                                           | **11,082 (83.3%)**                                                      |
| Soroban tx with 3+ events (partial coverage, likely via diagnostic_events)                                                                                        | 2,226 (16.7%)                                                           |
| Soroban tx with at least one "silent contract" — a contract appearing in `soroban_invocations_appearances` but absent from this tx's `soroban_events_appearances` | **6,154 / 6,230 = 98.8%** of Soroban tx that have any nested invocation |
| Distinct contracts called via invocations but with **zero** event rows anywhere in the DB                                                                         | 21 / 55 = **38%** of all Soroban-callable contracts                     |
| Total event rows currently in `soroban_events_appearances`                                                                                                        | 55,581 (mostly XLM SAC fee events)                                      |

The 83% "exactly 2 XLM-SAC events" cohort is the smoking gun: every
single one of those tx ran a Soroban contract that almost certainly
emitted at least one transfer/mint/burn/custom event during execution,
and none of those events made it into the DB. The two events present
are the Protocol 23 fee events (BeforeAllTxs charge + AfterTx refund)
which arrive at the tx-level `v4.events` location that we _do_ parse.

### Affected query consumers

- **`soroban_events_appearances` rows missed**: every Soroban contract
  event emitted during `InvokeHostFunction` execution (`transfer`,
  `mint`, `burn`, custom contract events). On the 100-ledger local
  sample, ballpark thousands of events missing.
- **`contract_ids[]` in E02 zwrotka (task 0167 / variant 2 of 02_get_transactions_list.sql)**:
  systematically incomplete for Soroban tx — root invocation present
  (from `operations_appearances`), nested calls present (from
  `soroban_invocations_appearances`), but contracts that _only emit
  events_ (don't appear in the call tree) are missed.
- **`/transactions/:hash` events tab (E03)**: shows tx-level fee +
  diagnostic events only; misses the actual Soroban execution events.
- **Search-by-contract (E02 statement B, post variant 2)**: typed-token
  searches return false negatives — e.g. searching for KALE SAC will
  not find `harvest`/`plant`/`work` tx unless the SAC also appeared as
  a nested call (which it sometimes does, sometimes doesn't).
- **Token holder analytics (task 0135 territory)**: any logic relying
  on transfer events for balance bookkeeping is broken on V4.
- **Classic-asset SAC events** (Protocol 23 unification): every classic
  Payment of XLM/USDC/etc. emits a SAC `transfer` event into
  `OperationMetaV2.events` that is dropped. Of 23,011 classic tx in DB,
  none have these events captured (only the tx-level fee event).

### Why this wasn't caught earlier

- Task 0026 (CAP-67 events) was implemented against V3 spec; V4 came
  later via Protocol 23. The branch was added but only for tx-level +
  diagnostic events, not the per-op location.
- Local test ledgers and integration tests pass because they were
  authored with V3-shape fixtures or V4 fixtures that happen to put
  events at tx-level.
- Indexer compiles + runs cleanly because the missing data is silent —
  there's no error, just fewer rows.

## Implementation Plan

### Step 1 — Extend `extract_events` for V4 per-op events

`crates/xdr-parser/src/event.rs::extract_events`, V4 branch:

```rust
TransactionMeta::V4(v4) => {
    let mut extracted: Vec<ExtractedEvent> = v4
        .events
        .iter()
        .enumerate()
        .map(|(i, tx_event)| extract_single_event(&tx_event.event, transaction_hash, ledger_sequence, created_at, i))
        .collect();

    // NEW: per-operation events (CAP-67 / Protocol 23+)
    let mut next_idx = extracted.len();
    for op_meta in v4.operations.iter() {
        for event in op_meta.events.iter() {
            extracted.push(extract_single_event(event, transaction_hash, ledger_sequence, created_at, next_idx));
            next_idx += 1;
        }
    }

    // diagnostic_events come last (existing behavior)
    for diag in v4.diagnostic_events.iter() {
        extracted.push(extract_single_event(&diag.event, transaction_hash, ledger_sequence, created_at, next_idx));
        next_idx += 1;
    }

    extracted
}
```

`event_index` numbering: sequential across all sources (tx-level →
per-op → diagnostic) preserves uniqueness and matches the V3 contract
where `event_index` is monotonic per-tx.

### Step 2 — Unit tests

In `crates/xdr-parser/src/event.rs::tests`:

1. **V4 with single per-op event** — build `TransactionMetaV4` with one
   `OperationMetaV2` carrying one `ContractEvent`, assert `extract_events`
   returns one row with the correct `contract_id`.
2. **V4 with mixed sources** — tx-level event + 2 per-op events (from
   2 different operations) + 1 diagnostic event. Assert order is
   tx-level → per-op (op0 then op1) → diagnostic, with sequential
   `event_index` 0, 1, 2, 3.
3. **V4 with empty per-op** — operations vec has entries but each
   `events` is empty. Assert no spurious rows.

### Step 3 — Integration verification on a small fixture

Add to `crates/indexer/tests/persist_integration.rs` (or a new test
file): build a synthetic V4 `LedgerCloseMeta` fixture with one
Soroban tx whose `OperationMetaV2.events` carries a known contract
event. Run the full ingest pipeline against the fixture and assert:

1. The fixture's contract appears in `soroban_events_appearances` for
   that tx.
2. `event_index` numbering is sequential across tx-level → per-op →
   diagnostic.
3. Existing V3 fixtures still produce the same row counts (no
   regression on the V3 path).

This is fixture-driven (no live data, no reindex). Reindex of the
local DB is the owner's call after the fix lands; the parser change
is self-contained and does not require it to validate correctness.

## Acceptance Criteria

- [ ] `extract_events` V4 branch iterates `v4.operations[i].events`,
      preserving sequential `event_index` numbering across tx-level →
      per-op → diagnostic.
- [ ] Unit tests cover the three V4 patterns (single per-op, mixed
      sources, empty per-op).
- [ ] Integration test on a synthetic V4 fixture asserts the per-op
      event lands in `soroban_events_appearances` after full ingest.
- [ ] V3 path (`SorobanTransactionMeta.events`) untouched and existing
      V3 tests still pass.
- [ ] **Docs updated** — per ADR 0032: - [ ] `docs/architecture/xdr-parsing/xdr-parsing-overview.md` §5.1
      (CAP-67 events) — explicit note on V3 vs V4 dispatch and
      per-op events location. - [ ] N/A for ADR 0033 (schema unchanged — same
      `soroban_events_appearances` table; only the population
      path changes). - [ ] N/A for ADR 0002 (already documented; ADR 0002 spec is
      correct, code was the gap).

## Issues Encountered (during root-cause analysis)

- **`SorobanTransactionMetaV2` no longer has `events` field** — initially
  I expected V4 to keep events on `soroban_meta` (V3 layout). Verifying
  the XDR struct in `stellar-xdr-26.0.0/src/curr/generated.rs` showed
  the field was removed; events relocated to `OperationMetaV2.events`
  and `TransactionMetaV4.events`. ADR 0002 captured this correctly;
  code did not follow.
- **Partial coverage masked the bug** — XLM SAC fee events appear in
  `v4.events` (tx-level) so `soroban_events_appearances` is non-empty
  for every classic XLM tx. Without a per-op-event reference, it looked
  like the data was complete but limited.

## Notes

- **Why this matters before mainnet backfill starts:** mainnet has been
  on Protocol 23 since Jun 2025 (Protocol 25 active by 2026-04).
  Owner has not yet kicked off the historical backfill; once it runs,
  this fix wants to already be in place so the ingested
  `soroban_events_appearances` index is correct from the first ledger
  rather than being silently incomplete on every Protocol ≥ 23 ledger.
- **Out of scope:**
  - Reindex / re-ingest of the local 100-ledger sample — owner's call
    after the parser is validated. The fix is parser-only and is
    fully verifiable from unit + integration tests on synthetic
    fixtures, no live ingest replay needed.
  - `is_sac` flag correctness on `soroban_contracts` — separate concern
    (task 0160 / 0161 territory). Flag for owner; do not absorb here.
  - Schema change to add an `event_source` column distinguishing
    tx-level vs per-op vs diagnostic — only do if read-time
    presentation needs it. Current consumers (E02 contract_ids[],
    E03 events tab) treat all events equally.
- **Repro tx for context** (no need to fetch from chain to validate the
  fix — fixture-based tests are sufficient — but useful as concrete
  examples of what's currently missing in the local DB):
  - `358ef42d9840d91554a46d69be7c7fee8f8f4305379ab6ed614e4ea9ae4e75dc`
    (KALE harvest) — KALE SAC mint event missing, only XLM SAC fee
    events captured.
  - `1c61a3b7b21ab48c6f02b72d124b4da86196091558d00c2879969d29a5ce2438`
    (Soroswap XLM↔USDC swap) — router-internal swap events missing,
    only fee + diagnostic-leaked events captured.
  - Any of the 11,082 Soroban tx with exactly 2 events in DB
    (`SUM(amount) = 2 FROM soroban_events_appearances`, all from
    XLM SAC) — these are the cleanest "100% execution events dropped"
    examples.
