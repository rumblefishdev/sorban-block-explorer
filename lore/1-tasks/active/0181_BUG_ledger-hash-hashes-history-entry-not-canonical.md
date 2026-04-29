---
id: '0181'
title: 'BUG: xdr-parser ledger.hash hashes the history entry, not the canonical ledger hash'
type: BUG
status: active
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

- [ ] `extract_ledger` emits `hex::encode(entry.hash.0)` (no SHA256)
- [ ] Unit test: synthetic meta with known entry hash → parser returns
      that hash exactly
- [ ] Fixture test (gated on `.temp/`): every ledger in a real batch
      round-trips its `entry.hash.0`
- [ ] DB sample (one ledger) hash equals Horizon `/ledgers/:N.hash`
      after re-ingest
- [ ] **Docs updated** — `docs/architecture/xdr-parsing/xdr-parsing-overview.md`
      §4.1 lists ledger hash extraction; confirm the line still describes
      the canonical hash (no shape change, may need wording fix only).
      `N/A — schema unchanged` for ADR 0033/0037.

## Notes

- Trivial fix (one-liner) but data-correctness wide blast radius —
  affects every `/ledgers` response.
- No schema change; column is already `BYTEA(32)` / hex-encoded string.
- Reindex is the owner's call: pre-existing rows hold the wrong value
  until re-ingest. Local 100-ledger sample re-ingest is a 30 s job.
