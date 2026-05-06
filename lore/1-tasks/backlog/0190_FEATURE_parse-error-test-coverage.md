---
id: '0190'
title: 'Test coverage gap: transactions.parse_error=true path never exercised'
type: FEATURE
status: backlog
related_adr: []
related_tasks: []
tags:
  [
    priority-low,
    effort-small,
    layer-indexer,
    layer-xdr-parser,
    testing,
    observability,
  ]
links:
  - crates/xdr-parser/src/transaction.rs
  - crates/indexer/tests/persist_integration.rs
  - crates/api/src/runtime_enrichment/stellar_archive/mod.rs
history:
  - date: '2026-05-05'
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned during /compare-with-stellar-api E03 (Statement B) verification.
      Sanity check on clone DB showed 0/10,118,806 transactions with
      parse_error=true. Code path is reachable (transaction.rs:133) but
      crates/indexer/tests/persist_integration.rs has 12 fixtures with
      parse_error=false and zero with true — DB persist + API Archive
      overlay handlers for parse_error rows are entirely untested.
---

# Test coverage gap: `transactions.parse_error=true` path never exercised

## Summary

The `transactions.parse_error` flag is reachable code, set by
`extract_single_transaction` in `crates/xdr-parser/src/transaction.rs:133`
when XDR re-serialization fails or the envelope can't be aligned with
`tx_processing`. In production we observe **0/10.1M** rows with
`parse_error=true` (canonical Stellar archive almost never trips the
conditions), but the test pyramid covers only the `false` path. Add unit
and integration coverage so the degraded-tx pipeline is validated, and
optionally an observability counter so silent accumulation triggers an
alert.

## Context

Discovered during `/compare-with-stellar-api` E03 verification (Statement B
header). Sanity check on the clone DB:

```sql
SELECT parse_error, COUNT(*) FROM transactions GROUP BY parse_error;
→ false | 10,118,806
→ true  | 0
```

Code review confirmed the flag is **not dead code**. Three reachable
triggers in `transaction.rs:133`:

1. `envelope.is_none()` — `extract_envelopes` failed to align `tx_set`
   with `tx_processing` (corrupt archive / hash mismatch). Warns
   `"envelope missing for transaction — parse_error"` at
   `transaction.rs:127`.
2. `envelope_xdr.is_empty()` — `encode_xdr` returned `""` after
   `to_xdr` failure, e.g. XDR `Limits` exceeded. Warns
   `"XDR serialization failed: {e}"` at `transaction.rs:166`.
3. `result_xdr.is_empty()` — same shape, for the `TransactionResult`
   blob.

The DB persist path and the API Archive-overlay handlers for
`parse_error=true` rows (`crates/api/src/runtime_enrichment/stellar_archive/mod.rs:241,326`,
which gate behind `if !t.parse_error` and `if tx.parse_error`) are not
exercised by any test. If a real parse failure ever lands in production,
its blast radius across persist + read paths is untested.

`crates/indexer/tests/persist_integration.rs` carries 12 fixtures with
`parse_error: false`, zero with `true`.

## Implementation Plan

### Step 1: `xdr-parser` unit test

Add a unit/integration test in `crates/xdr-parser/tests/` that constructs
a `LedgerCloseMeta` with intentionally bad input and asserts the failure
shape:

- Variant A — missing envelope: build a meta where `tx_set` lacks the
  envelope corresponding to a `tx_processing` entry (or hash mismatch
  defeats `extract_envelopes`); assert `ExtractedTransaction.parse_error
== true` and `envelope_xdr.is_empty()` and `source_account.is_empty()`.
- Variant B (optional) — encode failure: feed an envelope past
  `xdr_limits::serialization_limits()` and assert the same shape via
  the `encode_xdr` empty-string branch.

### Step 2: indexer integration fixture

Extend `crates/indexer/tests/persist_integration.rs` with at least one
`ExtractedTransaction { parse_error: true, … }` fixture:

- Empty `envelope_xdr`, `result_xdr` (mirrors real parse-failure shape).
- Empty `source_account` per `transaction.rs:129`.
- Assert persist completes without error and the row lands in
  `transactions` with `parse_error = true`.
- Assert downstream queries (E02 Statement A, E03 Statement B) return
  the row with the flag intact.
- Verify the API Archive-overlay path (`stellar_archive/mod.rs:326`)
  short-circuits on `parse_error=true` and surfaces only DB-only fields
  (no envelope/memo/signatures synthesis on degraded rows).

### Step 3 (optional): observability counter

Emit a `tracing` / Prometheus counter `parse_error_total` from the
indexer so production occurrences trigger an alert before they
accumulate silently. Suggested threshold: any non-zero increment in a
24-hour window pages oncall (these should be effectively never).

## Acceptance Criteria

- [ ] Unit test in `crates/xdr-parser/` asserts `parse_error=true` for
      at least one corrupt-input scenario (missing envelope or encode
      failure).
- [ ] Integration test in `crates/indexer/tests/persist_integration.rs`
      covers a `parse_error: true` fixture end-to-end (persist +
      downstream read).
- [ ] API `stellar_archive` overlay path verified to gracefully skip
      enrichment on `parse_error=true` rows.
- [ ] (Optional) Indexer emits a counter for `parse_error_total`.
- [ ] **Docs updated** — N/A: pure test + observability addition, no
      architectural shape change in `docs/architecture/**`.

## Notes

- Source DB during discovery: clone of `sbe-audit-postgres-1` snapshotted
  at ledger 62046000 (~10.1M transactions). Distribution:
  `parse_error=false` in 100% of rows.
- Root finding emerged via `/compare-with-stellar-api
docs/architecture/database-schema/endpoint-queries/03_get_transactions_by_hash.sql`
  Statement B sanity check.
- The flag is intentionally preserved (records never dropped on parse
  failure) per `crates/xdr-parser/src/transaction.rs:1-5` and
  `crates/xdr-parser/src/error.rs:1-5`. The downstream contract is "you
  may receive a transactions row whose XDR-derived fields are absent —
  handle it." That contract has no automated guarantor today.
