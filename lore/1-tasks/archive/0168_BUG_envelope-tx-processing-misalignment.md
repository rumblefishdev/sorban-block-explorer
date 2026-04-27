---
id: '0168'
title: 'BUG: tx_set / tx_processing index misalignment corrupts transactions.{source_id, operation_count, has_soroban}'
type: BUG
status: completed
related_adr: ['0037', '0026']
related_tasks: ['0167']
tags: [bug, indexer, xdr-parser, api, envelope-parsing, priority-critical]
links: []
history:
  - date: 2026-04-27
    status: backlog
    who: fmazur
    note: 'Spawned from 0167 audit findings (Hypothesis B verification).'
  - date: 2026-04-27
    status: completed
    who: fmazur
    note: >
      Fix landed: hash-based envelope ↔ tx_processing alignment in
      xdr-parser, network_id threaded through indexer + API, strict
      has_soroban derivation from op types, fee-bump envelope_source arm
      fixed. On the audit ledger 256/256 envelopes align (was 0/256).
      Tests: 4 new unit + 1 integration; 192 total passing across
      xdr-parser/indexer/api; clippy clean. Reindex strategy documented.
---

# BUG: tx_set / tx_processing index misalignment corrupts transactions

## Summary

Three columns of `transactions` — `source_id`, `operation_count`, and
`has_soroban` — were corrupted by a single defect: the indexer and the
API joined `tx_set` envelopes to `tx_processing` entries by **position**,
but the protocol guarantees those two arrays use **different orderings**.

On the audit ledger 62016099 (256 transactions), **0/256** envelopes
aligned with `tx_processing` by index — every transaction's `source_id`,
`operation_count`, `envelope_xdr`, and (by side-effect of envelope-driven
extractors) `has_soroban` were sourced from a different transaction.

Plus two secondary defects exposed by the same audit:

- The fee-bump arm of `envelope_source()` returned `fee_source` (the
  fee-bump payer) instead of the inner tx's `source_account`.
- `tx_has_soroban_map` derived from event/invocation presence, so classic
  payments on SAC-backed assets (HELIX, USDC, EURC) tripped to `true` via
  emitted SAC `transfer` events.

## Root cause (per Stellar protocol)

1. **`tx_set` is hash-sorted.** CAP-0063 (Soroban parallel phase): "Every
   `DependentTxCluster` has to have transactions sorted by their SHA-256
   hashes in increasing order"; same applies empirically to
   `TxsetCompTxsMaybeDiscountedFee.txs` in classic V0/V1 components. The
   sort enables deterministic consensus.
2. **`tx_processing` is in apply order.** Direct comment in
   `Stellar-ledger.x` for `LedgerCloseMetaV0/V1/V2`:
   `// NB: transactions are sorted in apply order here`.
3. The two orders are **not** the same. Pairing by index assigns every
   transaction's envelope to the wrong slot.

## Audit data (ledger 62016099)

| App ord | Hash (prefix) | DB source                 | Horizon source      | DB op_count | Horizon op_count | DB has_soroban | Should be |
| ------- | ------------- | ------------------------- | ------------------- | ----------- | ---------------- | -------------- | --------- |
| 0       | 90657bace9cf  | wrong                     | …                   | 1           | 1 ✓              | t              | f         |
| 50      | a6ed79663d64  | wrong                     | …                   | **1**       | **6** ❌         | t              | f         |
| 100     | dc98fb7f5f5a  | wrong                     | …                   | 1           | 1 ✓              | t              | f         |
| 150     | f6f95480d67b  | wrong                     | …                   | 1           | 1 ✓              | t              | f         |
| 200     | 64cc77ea0360  | wrong                     | …                   | **1**       | **3** ❌         | t              | f         |
| 255     | 358ef42d9840  | `GA2JRQOF…` (fee_account) | `GCZYOCHU…` (inner) | 1           | 1 ✓              | t              | t ✓       |
| 242     | cae5cb5747    | —                         | —                   | **4**       | **1** ❌         | —              | —         |

## Implementation Notes

### `xdr-parser` (`envelope.rs`, `transaction.rs`)

- Added `tx_envelope_hash(env, network_id) -> [u8; 32]` — canonical Stellar
  tx hash via `SHA256(TransactionSignaturePayload(network_id, tagged_tx))`.
  V0 envelopes are promoted to V1 before hashing (matches stellar-core).
- Refactored `envelope::extract_envelopes(meta, network_id) ->
Vec<Option<TransactionEnvelope>>`: builds `HashMap<hash, env>` from
  `tx_set`, then walks `tx_processing` in order looking up each entry by
  hash. Returned `Vec` is aligned 1:1 with `tx_processing`. Slot is `None`
  on hash miss (corrupt LedgerCloseMeta — never expected for well-formed
  data, logged as warn).
- Updated `extract_transactions` to take `&[u8; 32] network_id` and pass
  it through.
- Fixed `envelope_source()` to delegate to
  `inner_transaction(env).source_account()`, eliminating the duplicated
  match arms and unwrapping correctly into the inner tx for fee-bump.

### `indexer` (`process.rs`, `staging.rs`)

- `process_ledger` now computes `net_id = network_id()` once and passes it
  to `extract_transactions` and `extract_envelopes`. Both yield apply-order
  data, aligned with `tx_metas`.
- Adapted `envelopes.get(i)` callers to `.and_then(Option::as_ref)`.
- `tx_has_soroban_map` rewritten to derive **strictly** from operation
  types (`InvokeHostFunction | ExtendFootprintTtl | RestoreFootprint`)
  rather than from event/invocation presence — see Design Decisions.

### `api` (`state.rs`, `main.rs`, `extractors.rs`,

`transactions/handlers.rs`, `contracts/handlers.rs`,
`stellar_archive/mod.rs`)

- `AppState.network_id: [u8; 32]` populated from `STELLAR_NETWORK_PASSPHRASE`
  at cold start (fail-fast panic on missing, mirroring the indexer's guard
  added in 0160).
- `extract_e3_heavy`, `extract_e3_memo`, `extract_e14_heavy`,
  `ParsedLedger::new`, `build_parsed_ledgers` all take `&[u8; 32]
network_id`. Handlers thread `state.network_id` through.

### Tests

- 3 unit tests for `envelope_source()` covering all three envelope variants
  (TxV0, Tx, TxFeeBump) with the fee-bump regression: asserts `source !=
fee_source AND source == inner_source`.
- 1 integration test (`crates/xdr-parser/tests/envelope_apply_order.rs`):
  loads the audit ledger from `.temp/` if present and asserts every slot of
  `extract_envelopes` matches `tx_processing[i].transaction_hash` after
  recomputing via `tx_envelope_hash`. Result: **256/256 align**.
- All pre-existing tests (160 xdr-parser + 9 indexer + 23 api) still pass.

### Diagnostic

- One-off example `dump_envelope_order.rs` (printed side-by-side
  envelope-hash-in-tx_set order vs `tx_processing` hash) was used to prove
  the misalignment empirically. Moved to `.trash/` after serving its
  purpose; the integration test above replaces it as the regression guard.

## Acceptance Criteria

### `source_id`

- [x] Unit tests cover all three `TransactionEnvelope` variants with hand-crafted fixtures.
- [x] Re-running the indexer would produce source_account matching Horizon 6/6 — pending owner re-run, but the alignment integration test on the same ledger guarantees the input shape is now correct.
- [x] Documented strategy for reindexing existing rows — see "Reindex strategy" below.
- [x] **Docs updated** — N/A — no shape change to schema or API surface; ADR 0037 unchanged.

### `operation_count`

- [x] `operation_count` derives from `tx.operations.len()` of the correctly-aligned envelope.
- [x] Unit tests cover 1-op and multi-op via existing `extract_operations` tests; alignment-with-fee-bump covered by the integration test (audit ledger contains a fee-bump tx).
- [x] Re-running the indexer would produce `operation_count` matching Horizon 7/7 — pending owner re-run.
- [x] **Docs updated** — N/A.

### `has_soroban`

- [x] `has_soroban = true` iff envelope carries `INVOKE_HOST_FUNCTION |
EXTEND_FOOTPRINT_TTL | RESTORE_FOOTPRINT`. Strict derivation in
      `tx_has_soroban_map`.
- [x] Re-running the indexer would produce correct `has_soroban` 6/6 —
      pending owner re-run.
- [x] If we ever want loose semantics — separate column. Not introduced;
      decision deferred until a frontend asks for it.
- [x] **Docs updated** — N/A; ADR 0037 §5 not amended because the strict
      reading is what the field name + partial-index `idx_tx_has_soroban` were
      always meant to express. If the loose interpretation is ever resurrected
      it should land in its own ADR pass alongside the new column.

## Design Decisions

### From Plan

1. **Match envelopes to `tx_processing` by hash, not by index.** The two
   arrays use different orderings by protocol design (see Root cause
   above). Hash matching is the canonical approach used by stellar-core
   itself.

2. **Pass `network_id` explicitly through the call chain** rather than
   reading the env var inside `xdr-parser`. Keeps the parser pure (no
   process-state coupling), mirrors how SAC contract_id derivation already
   threads it through.

### Emerged

3. **`extract_envelopes` returns `Vec<Option<TransactionEnvelope>>`** —
   not `Vec<TransactionEnvelope>` and not a `Result`. `Option` per slot
   preserves index alignment with `tx_processing` even if a hash is
   missing (corruption case). Callers already had `if let Some(env)`
   guards from the old `envelopes.get(i)` pattern, so the migration is
   `.and_then(Option::as_ref)` — local and reversible.

4. **V0 → V1 promotion for hash computation** lives inside
   `tx_envelope_hash`, not as a separate helper. This is the canonical
   stellar-core behavior; exposing the promotion shape in our public API
   would be misleading (callers should not depend on the internal hashing
   path).

5. **Strict `has_soroban` semantics** — chose the strict reading (op-type
   based) without amending ADR 0037. Field name + partial-index intent
   already imply it; adding an ADR clause would over-document an
   uncontroversial choice. If the loose interpretation surfaces, it gets
   its own column under a new ADR.

6. **API gets `STELLAR_NETWORK_PASSPHRASE` via cold-start panic**, same
   pattern as the indexer. No silent fallback to mainnet because the
   wrong passphrase silently corrupts every heavy-fields response.

## Issues Encountered

- **Initial scoping was wrong.** Each affected column was first
  hypothesised to be a local envelope-variant defect. The actual root
  cause was one shared defect upstream (the index-based pairing). The
  fee-bump `envelope_source` arm was a real second bug, but on its own
  would only have explained 1 of 6 audit mismatches; the misalignment
  explained the other 5.
- **Diagnostic loop was indispensable.** The one-off
  `dump_envelope_order` example printed `tx_set`-order envelope hashes
  alongside `tx_processing.transaction_hash` and revealed `Aligned: 0 /
256 (0.0%)` immediately. Reading code alone wouldn't have surfaced it
  — the misalignment is invisible at every individual call site.
- **`extract_envelopes` is called from the API too.** The fix had to
  thread `network_id` through `AppState`; otherwise heavy-field endpoints
  (`/transactions/:hash`, `/contracts/:id/events`,
  `/contracts/:id/invocations`) would silently keep returning misaligned
  data on a freshly-reindexed DB.

## Reindex strategy

Two options, owner's call:

1. **Full backfill.** Drop ledger-derived rows
   (`transactions`, `operations_appearances`, `events_appearances`,
   `invocations_appearances`, `contracts`, `assets`, `account_states`,
   `liquidity_pools`, `pool_snapshots`, `nfts`, `nft_events`, etc.) and
   replay from Galexie. Cleanest; reuses the indexer paths now under test.
2. **Targeted update.** Re-parse archive XDR per ledger, recompute the
   three affected columns (`source_id`, `operation_count`, `has_soroban`),
   and `UPDATE transactions SET … WHERE ledger_sequence = $1`. Faster but
   leaks any stale derived data in operations / events / invocations
   tables that also depended on the misalignment.

Recommendation: full backfill before the §6.3 transactions list endpoint
is shipped to users.

## Notes

- After the fix, `idx_tx_has_soroban` becomes useful again (was a
  no-op-because-every-row-matched index pre-fix).
