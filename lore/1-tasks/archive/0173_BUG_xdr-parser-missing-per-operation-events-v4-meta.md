---
id: '0173'
title: 'BUG: xdr-parser drops per-operation events in V4 meta (CAP-67 / Protocol 23+)'
type: BUG
status: completed
related_adr: ['0002', '0033']
related_tasks: ['0167', '0026', '0181', '0182']
tags: [xdr-parser, cap-67, events, priority-critical, pre-mainnet-backfill]
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
  - date: '2026-04-29'
    status: completed
    who: fmazur
    note: >
      Implemented parser fix + emerged staging defense-in-depth filter.
      4 files modified: crates/xdr-parser/src/event.rs (V4 per-op iteration
      + 3 unit tests + comprehensive ordering/indexing test),
      crates/indexer/src/handler/persist/staging.rs (events transfer-
      participants filter + account_keys_set defense-in-depth filter for
      VARCHAR(56) overflow regression — see Issues), crates/indexer/tests/
      persist_integration.rs (end-to-end V4 per-op test through
      extract_events → persist_ledger), docs/architecture/xdr-parsing/
      xdr-parsing-overview.md §5.1 (V3↔V4 dispatch + 3-location
      enumeration). Spawned follow-ups: 0181 (ledger.hash bug found
      during E02 cross-validation against Horizon), 0182 (diagnostic_events
      container leak — Contract-typed mirror duplicates overcount amount
      ~2.5×, found during E02 cross-validation against stellar.expert).
      173 unit tests + 10 persist_integration tests passing; backfill
      62016000-62016099 runs clean (228 ms mean, p99 364 ms);
      SUM(amount) in soroban_events_appearances 55,581 → 92,335 (+66%).
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

## Status: Completed

**Current state:** Parser fix + staging defense-in-depth filter shipped.
Backfill 62016000-62016099 runs clean. Two follow-up bugs spawned and
documented as separate tasks (0181 ledger.hash, 0182 diagnostic_events
container leak).

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

- [x] `extract_events` V4 branch iterates `v4.operations[i].events`,
      preserving sequential `event_index` numbering across tx-level →
      per-op → diagnostic.
- [x] Unit tests cover the three V4 patterns (single per-op, mixed
      sources, empty per-op).
- [x] Integration test on a synthetic V4 fixture asserts the per-op
      event lands in `soroban_events_appearances` after full ingest
      (`v4_per_op_events_land_in_appearance_index` in
      `crates/indexer/tests/persist_integration.rs`).
- [x] V3 path (`SorobanTransactionMeta.events`) untouched and existing
      V3 tests still pass (10/10 persist_integration + 173/173 xdr-parser
      unit tests green).
- [x] **Docs updated** — per ADR 0032:
  - [x] `docs/architecture/xdr-parsing/xdr-parsing-overview.md` §5.1
        (CAP-67 events) — added "V3 vs V4 meta dispatch" subsection
        with explicit 3-location enumeration for V4 + symptom callout
        for the "exactly 2 fee events" cohort.
  - [x] N/A for ADR 0033 (schema unchanged — same
        `soroban_events_appearances` table; only the population
        path changes).
  - [x] N/A for ADR 0002 (already documented; ADR 0002 spec is
        correct, code was the gap).

## Implementation Notes

**Files modified (4):**

1. `crates/xdr-parser/src/event.rs` — V4 branch in `extract_events`
   gained a per-op iteration step between the existing tx-level and
   diagnostic loops. Sequential `event_index` numbering preserved via a
   single `next_idx` counter shared across the three sources. Added
   3 unit tests pinning each pattern (single per-op, mixed sources with
   ordering+indexing assertions, empty per-op operations).
2. `crates/indexer/src/handler/persist/staging.rs` — two filter changes
   (see Design Decisions ### Emerged): events transfer-participants
   push gained `is_strkey_account` filter (matching invocations/ops
   paths); `account_keys_set` finalization gained a defense-in-depth
   length+prefix filter dropping non-G-56 strkeys with aggregated
   debug log.
3. `crates/indexer/tests/persist_integration.rs` — added end-to-end
   V4 per-op test (`v4_per_op_events_land_in_appearance_index`) that
   builds a synthetic `TransactionMeta::V4` (1 tx-level Contract event +
   2 per-op Contract events on a single op + 1 Diagnostic), runs through
   `extract_events` → `persist_ledger`, asserts 1 appearance row with
   `amount = 3` (Diagnostic filtered at staging). Pre-fix this would
   have produced `amount = 1` (only the tx-level event).
4. `docs/architecture/xdr-parsing/xdr-parsing-overview.md` — §5.1
   "V3 vs V4 meta dispatch" subsection with explicit 3-location
   enumeration (tx-level / per-op / diagnostic) and the "silent 2-event
   cohort" symptom signature.

**Empirical validation:**

- xdr-parser unit tests: 173/173 passing.
- indexer persist_integration tests: 10/10 passing (DB-gated, run
  against local Postgres).
- Backfill 62016000-62016099 (100 ledgers, 36,319 tx total): runs
  clean, 99 indexed + 1 skipped, no errors. Mean 228 ms/ledger,
  p99 364 ms.
- `SUM(soroban_events_appearances.amount)` on the 100-ledger window:
  pre-fix 55,581 → post-fix 92,335 (+66% / +36,754 events surfaced
  that were previously dropped at the `OperationMetaV2.events`
  location).
- Cross-validation against Horizon API: 5 sample tx (4 successful
  Soroban with mixed fee-bump shapes + 1 failed) byte-identical on
  hash / ledger_sequence / source_account / fee_charged / successful /
  operation_count / inner_tx_hash / created_at / application_order
  (decoded from Horizon TOID `(paging_token >> 12) & 0xFFFFF`).
- Cross-validation against stellar.expert API: per-tx event counts
  match per-op consensus events exactly for the Soroswap swap repro tx
  (revealed task 0182 — diagnostic-container Contract mirrors leak
  through current type-based filter and overcount `amount` ~2.5×).

## Design Decisions

### From Plan

1. **V4 iteration order**: tx-level → per-op → diagnostic, with a
   single `next_idx` counter incremented across all three sources.
   Preserves the V3 contract that `event_index` is monotonic per-tx
   so downstream consumers (E03, E14 read-time API; SMALLINT bound
   in API DTO) keep their invariants.

2. **No V3 changes**: V3 spec keeps events at
   `soroban_meta.events` with no per-op location. V3 branch in
   `extract_events` left untouched; existing V3 fixtures and tests
   continue to pass unchanged.

### Emerged

3. **Staging events transfer-participants filter** (out of plan).
   The plan was parser-only, but real backfill against ledger
   62016000 surfaced an immediate regression: per-op SAC events from
   classic operations on claimable balances carry topic `Address`
   values rendered as `B…` ClaimableBalance StrKeys (58 chars) which
   overflow `accounts.account_id VARCHAR(56)`. Diagnostic at
   `crates/xdr-parser/tests/diag_v4_oversize_strkey.rs` (since moved
   to `.trash/`) found 86 instances on a single tx in ledger 62016000.
   Added `is_strkey_account` filter to the events transfer-participants
   push in `staging.rs:312-322` to match the existing
   invocations/operations participant filtering — drops B/L addresses
   independently per side so a `(B, G)` transfer still tracks G.

4. **Defense-in-depth `account_keys_set` finalization filter** (out
   of plan). Filter (3) addressed the events path, but other paths
   (ops, tx.source_account, account_states issuers, NFT events) carry
   the same latent risk for M-muxed (`MuxedAccount::MuxedEd25519`,
   69 chars). Added a single chokepoint at
   `staging.rs:401-422` that drops anything not 56-char G-prefix
   before the accounts insert, with an aggregate `tracing::debug!`
   per-ledger so a high-leak ledger does not spam the trace. Docs
   the rationale; non-G addresses still flow through participants_per_tx
   and harmlessly fall out at write-time when the strkey doesn't
   resolve in `account_ids` map. M-muxed canonicalization to
   underlying G is intentionally out of scope — flagged for separate
   task per backlog 0177 (muxed-account leak into persist).

5. **Comments preserved on parser fix**. Project CLAUDE.md says
   default to no comments, but V4 per-op events are non-obvious
   (CAP-67 reorganisation). Kept short comment block on V4 branch
   matching the existing V3 explanatory comment style; defers to
   docs §5.1 for the full enumeration.

## Future Work

Spawned as separate backlog tasks during cross-validation:

- **0181** — `BUG: xdr-parser ledger.hash hashes the history entry,
not the canonical ledger hash`. Pre-existing; surfaced when E04/E05
  validation showed `ledgers.hash` not matching Horizon. Trivial
  one-liner fix.
- **0182** — `BUG: diagnostic_events container leak — Contract-typed
mirrors overcount soroban_events_appearances.amount ~2-3x`.
  Direct follow-up: Stellar core mirrors every consensus Contract
  event from `v4.operations[i].events` into `v4.diagnostic_events`
  (byte-identical), which our type-based staging filter does not
  catch. Need to switch from inner-type filter to container-source
  filter (introduce `EventSource` enum on `ExtractedEvent`).

## Issues Encountered

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

- **VARCHAR(56) regression on first backfill (post-fix)** — running
  the unmodified pipeline against ledger 62016000 with the new per-op
  iteration crashed with `ERROR 22001: value too long for character
varying(56)` on the accounts insert. Diagnostic localized 86
  oversize StrKeys on a single tx, all `B…` ClaimableBalance addresses
  (58 chars each) carried in topic `Address` of CAP-67 SAC unification
  events on classic claimable-balance ops. Root cause: the events
  transfer-participants push in staging was unfiltered (other paths
  used `is_strkey_account`), and `is_strkey_account` itself accepts
  `M…` muxed addresses (69 chars) which would also overflow.
  Resolved with the two emerged filters under Design Decisions
  (events-side `is_strkey_account` + finalization defense-in-depth).
  Not a regression vs pre-task-0173 behavior — pre-fix path didn't
  surface these events at all, so the latent bug was never hit;
  surfacing per-op events reveals the chain of accountability that
  was always supposed to be filtered.

- **Modified test scope clarification** — `extract_events_from_v4_meta`
  test was already in place pre-task and remains unchanged (it tests
  V4 with only tx-level events). The 3 new V4 unit tests
  (`extract_events_v4_per_op_single`, `_mixed_sources_preserve_order_and_indexing`,
  `_empty_per_op_produces_no_spurious_rows`) are additive — no
  existing V3 or V4 tests were modified.

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
