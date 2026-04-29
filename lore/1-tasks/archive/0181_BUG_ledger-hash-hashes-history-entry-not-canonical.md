---
id: '0181'
title: 'BUG: xdr-parser ledger.hash hashes the history entry, not the canonical ledger hash'
type: BUG
status: completed
related_adr: []
related_tasks: ['0173']
tags: ['xdr-parser', 'ledger', 'data-correctness', 'effort-small']
links: []
history:
  - date: '2026-04-29'
    status: backlog
    who: fmazur
    note: >
      Spawned from task 0173 cross-validation against Horizon
      (`docs/architecture/database-schema/endpoint-queries` E02 verification).
      Bug pre-dates 0173 — flagged independently while comparing
      `ledgers.hash` between DB and Horizon.
  - date: '2026-04-29'
    status: active
    who: fmazur
    note: 'Promoted to active via /promote-task to unblock E04/E05 ledger hash correctness.'
  - date: '2026-04-29'
    status: done
    who: claude
    note: >
      Replaced SHA256(header_xdr) with hex(entry.hash.0). Function made
      infallible (signature: Result<ExtractedLedger,_> → ExtractedLedger);
      6 callsites updated. 2 new unit tests (synthetic V0 LedgerCloseMeta
      with controlled hash); 1 new fixture-based integration test that
      asserts canonical-hash round-trip per ledger in a real Galexie
      batch. After test backfill, all 100 DB rows (62016000-62016099,
      protocol 25) match Horizon /ledgers/:N.hash byte-for-byte (verified
      programmatically). Docs: types.rs comment + xdr-parsing-overview.md
      §4.1 updated to describe canonical hash sourcing. clippy + workspace
      build clean.
---

# BUG: xdr-parser ledger.hash hashes the history entry, not the canonical ledger hash

## Summary

`crates/xdr-parser/src/ledger.rs::extract_ledger` computes
`SHA256(LedgerHeaderHistoryEntry XDR)` and stores the result in
`ExtractedLedger.hash`. The canonical Stellar ledger hash that every
other tool publishes (Horizon `/ledgers/:N.hash`, stellarchain.io,
stellar.expert) is the `hash` field of `LedgerHeaderHistoryEntry`,
already populated by core. As a result `ledgers.hash` in our DB does
not round-trip with any other Stellar tool.

## Context

Surfaced during task 0173 verification of E02 zwrotka against Horizon.
For ledger 62016099:

- DB `ledgers.hash`: `1f0c9b146ccfb2134eb3e177ef00c3fff23993c2a55f497b80e985fa0e282ac8`
- Horizon `/ledgers/62016099.hash`: `028ba5b6c2b0f3ad8e1acd08a288c2fc4a06034035ff072de29ee4d1a3eb49e2`

The canonical value is already on `LedgerHeaderHistoryEntry.hash` —
no recomputation needed.

Affected endpoints (E04 `GET /ledgers`, E05 `GET /ledgers/:sequence`)
return a hash no other explorer recognises; users copying it to
cross-reference get no match.

## Implementation Plan

### Step 1 — Use `entry.hash.0` directly

`crates/xdr-parser/src/ledger.rs:11-30`:

```rust
// Replace SHA256(header_entry XDR) with the already-populated
// canonical ledger hash field.
let hash = hex::encode(header_entry.hash.0);
```

Drop the `Sha256` import + the `to_xdr` + `Sha256::digest` chain and
the matching `XdrSerializationFailed` error path (the canonical hash
extraction is infallible).

### Step 2 — Tests

In `crates/xdr-parser/src/ledger.rs::tests` (or a new test file):

1. **Canonical hash matches `entry.hash.0`** — build a synthetic
   `LedgerCloseMeta` with a known `LedgerHeaderHistoryEntry.hash` and
   assert `extract_ledger(&meta).hash == hex::encode(entry.hash.0)`.
2. **Fixture-based round-trip (gated)** — load `.temp/<ledger>.xdr.zst`
   if present and assert the parser hash matches `entry.hash.0` for
   every meta in the batch (skip cleanly when fixture absent).

### Step 3 — Reindex / migration consideration

Since the column type does not change (still 32-byte hex string in DB),
existing rows can be left as-is or backfilled by re-ingesting the
ledgers — owner's call. No schema migration needed.

## Acceptance Criteria

- [x] `extract_ledger` emits `hex::encode(entry.hash.0)` (no SHA256)
- [x] Unit test: synthetic meta with known entry hash → parser returns
      that hash exactly — `crates/xdr-parser/src/ledger.rs::tests`
      (2 tests: incrementing-byte pattern + 0xAB-pattern guard)
- [x] Fixture test (gated on `.temp/`): every ledger in a real batch
      round-trips its `entry.hash.0` — new
      `crates/xdr-parser/tests/ledger_hash_canonical.rs`
- [x] DB sample (one ledger) hash equals Horizon `/ledgers/:N.hash`
      after re-ingest — verified all 100 DB rows
      (62016000-62016099) match Horizon byte-for-byte after test
      backfill (programmatic comparison, 0 mismatches / 0 errors)
- [x] **Docs updated** — `docs/architecture/xdr-parsing/xdr-parsing-overview.md`
      §4.1 amended to list `hash` as canonical-from-core (was missing
      from the bullet list entirely). `crates/xdr-parser/src/types.rs`
      doc-comment for `ExtractedLedger.hash` rewritten to drop the
      "SHA-256 of XDR" wording. `N/A — schema unchanged` for ADR 0033/0037.

## Implementation Notes

- **Fix** (`crates/xdr-parser/src/ledger.rs`): replaced fallible
  `to_xdr → Sha256::digest → hex::encode` chain with a single
  `hex::encode(header_entry.hash.0)`. Dropped the `XdrSerializationFailed`
  error production from this function (variant kept in `error.rs` —
  still produced by `sac.rs`).
- **Signature change**: `extract_ledger` now returns `ExtractedLedger`
  directly instead of `Result<ExtractedLedger, ParseError>`. All other
  field reads are infallible by construction, so the `Result` wrapper
  was deceptive after removing the SHA256 chain.
- **Callsite updates** (6 sites):
  - `crates/api/src/stellar_archive/extractors.rs:32,127,146` —
    dropped `.ok()?` and `match` arms
  - `crates/api/src/stellar_archive/mod.rs:237,317` — dropped
    `.unwrap()`
  - `crates/api/src/contracts/handlers.rs:137` — dropped
    `match { Ok(l) => l, Err(e) => { warn!(...); None } }`
  - `crates/indexer/src/handler/process.rs:56` — dropped `?`
- **Tests**: 2 unit tests in `ledger.rs::tests` build a synthetic
  `LedgerCloseMetaV0` with a caller-controlled `entry.hash` and assert
  the parser surfaces that hash verbatim. 1 integration test
  (`tests/ledger_hash_canonical.rs`) iterates the real Galexie batch
  in `.temp/` (gated on `XDR_FIXTURE` / file existence, skips cleanly
  in CI) and asserts canonical round-trip per ledger.
- **Build hygiene**: `cargo build --workspace --tests` and
  `cargo clippy -p xdr-parser -p api -p indexer --tests` clean.

## Design Decisions

### From Plan

1. **Use `entry.hash.0` directly, no recomputation** — exactly the
   plan's Step 1. Canonical hash is already on the
   `LedgerHeaderHistoryEntry` from stellar-core; recomputing it was
   the bug.

2. **Fixture-gated integration test** — plan's Step 2 (2): real-data
   round-trip for every meta in a `.temp/` batch, skipped cleanly when
   absent.

### Emerged

3. **Signature changed from `Result<ExtractedLedger, ParseError>` to
   `ExtractedLedger`** — the plan said "drop the matching
   `XdrSerializationFailed` error path" but didn't explicitly call
   for a signature change. Once the only fallible operation was gone,
   the `Result` wrapper was misleading: callers were forced to handle
   an `Err` that could never be produced, generating dead branches at
   every callsite. Owner approved the refactor over option A
   (keep `Result`, only return `Ok`) before implementation.

4. **Synthetic unit test built from scratch (not via fixture
   mutation)** — initially considered loading a fixture and mutating
   `entry.hash.0` to dodge the verbose `LedgerHeader` constructor.
   Chose explicit `LedgerCloseMetaV0` construction so the unit test
   has zero fixture dependency and can run in CI. The boilerplate
   (`StellarValue`, `LedgerHeaderExt`, etc.) was a one-time cost.

5. **`docs/architecture/xdr-parsing/xdr-parsing-overview.md` §4.1
   gained a new `hash` bullet** — the AC said "may need wording fix
   only", but the section in fact omitted `hash` entirely (listed
   `txSetResultHash` from the inner `LedgerHeader`, never the
   canonical entry hash). Adding the bullet aligns docs with both
   reality post-fix and the field's prominence in API responses.

6. **Verified all 100 DB rows, not just one** — AC asked for "one
   ledger". Tightened to programmatic comparison of every row in the
   62016000-62016099 backfill window (zero mismatches) since the
   margin cost was small and the data-correctness blast radius is
   wide.

## Issues Encountered

- **`Hash` is not `Copy`** — initial unit test wrote
  `skip_list: [Hash([0; 32]); 4]` but `stellar-xdr` doesn't impl
  `Copy` on `Hash`. Resolved by spelling out the four elements
  explicitly. Not a regression — just a stellar-xdr type constraint.

## Future Work

- **Production reindex** — owner's call. Local 100-ledger sample is
  fully backfilled and verified canonical (mainnet, protocol 25).
  Production rows ingested before the fix still hold the buggy
  recomputed digest until re-ingest. No backlog task spawned per
  user preference; tracked here for owner.
- **V0/V1 `LedgerCloseMeta` real-data check** — DB and the available
  `.temp/` fixture are entirely protocol-25 (V4 meta inside a V2
  variant). Fix is variant-agnostic by inspection (`ledger_header_entry`
  matches V0/V1/V2 symmetrically and reads the same `hash` field), and
  the synthetic unit test exercises V0; but no real V0/V1 batch was
  exercised end-to-end. Low risk given the structural argument; flag
  here for completeness.

## Notes

- Trivial fix (one-liner) but data-correctness wide blast radius —
  affects every `/ledgers` response.
- No schema change. `ledgers.hash` column is `BYTEA` (32 raw bytes,
  enforced by `ck_ledgers_hash_len`). The parser still emits a 64-char
  hex `String` on `ExtractedLedger.hash`; the persistence layer
  hex-decodes to `BYTEA` at insert time.
