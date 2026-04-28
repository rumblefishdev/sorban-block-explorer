---
id: '0169'
title: 'inner_tx_hash population + operations_appearances.amount semantics audit'
type: BUG
status: completed
related_adr: ['0037']
related_tasks: ['0167', '0168', '0163']
tags: [indexer, schema, api, fee-bump, soroban, audit]
links:
  - 'docs/architecture/database-schema/endpoint-queries/02_get_transactions_list.sql'
  - 'lore/1-tasks/archive/0167_FEATURE_endpoint-sql-query-reference-set.md'
  - 'lore/1-tasks/archive/0168_BUG_envelope-tx-processing-misalignment.md'
history:
  - date: 2026-04-28
    status: backlog
    who: fmazur
    note: >
      Spawned from manual E02 verification. Two findings against Horizon
      mainnet on ledger 62016099 that need investigation + potential fix.
  - date: 2026-04-28
    status: active
    who: fmazur
    note: 'Promoted to active via /promote-task'
  - date: 2026-04-28
    status: completed
    who: fmazur
    note: >
      Both findings shipped. F1: parser computes inner_tx_hash for
      fee-bumps via `inner_tx_hash(env, network_id)`; verified byte-for-byte
      against stellar-core's `InnerTransactionResultPair.transaction_hash`
      on 34/34 fee-bumps in audit ledger 62016099. F2: primary-op preview
      removed from E02 (8 columns + LATERAL + 4 LEFT JOINs); `oa.amount`
      removed from E03 per-op projection. Tests 170 + 9 integration +
      2 new unit + 1 new integration end-to-end. Local reindex confirmed
      `inner_tx_hash` now populated for the audit row in DB.
---

# inner_tx_hash population + operations_appearances.amount semantics audit

## Summary

Two findings surfaced during manual verification of the
`GET /transactions` endpoint against Horizon mainnet. Each may be a real
indexer/schema bug or a deliberate-but-undocumented choice; this task
investigates both, decides, and either fixes or documents.

## Context

E02 was verified on 50 rows from ledger `62016099`. 4 rows hand-checked
against Horizon: 3 fully aligned, 1 (the fee-bump audit row
`358ef42d…`) revealed two remaining discrepancies after lore-0168.

### Finding 1 — `transactions.inner_tx_hash` NULL on fee-bump rows

For the audit row `358ef42d9840d91554a46d69be7c7fee8f8f4305379ab6ed614e4ea9ae4e75dc`:

- **Horizon** reports `is_fee_bump_transaction = true`, `inner_transaction_hash = 12021959a49f62ec43b6985a22682ca63104c4a99641a1f83e0986baf15b266d`.
- **DB** has `inner_tx_hash = NULL`.

lore-0168 fixed `source_id` for fee-bumps (the inner-tx source is now
correct) but did not touch `inner_tx_hash` population. The column shape
in ADR 0037 implies it should be set when the row IS a fee-bump
envelope. Need to confirm whether the indexer's
`extract_transactions` path computes the inner hash for fee-bump
variants, or silently leaves it `NULL`.

### Finding 2 — `operations_appearances.amount` dual semantics

E02's projection `primary_op_amount = pop.amount` is `1` on every
soroban INVOKE_HOST_FUNCTION row in the sample. That's not stroops
— per ADR 0037 §7 / task 0163 the column is the **fold count of
duplicate appearances**, not a value amount. The column is reused
across op types with different meaning:

- classic transfer ops: stroop amount (real value).
- soroban / appearance-only rows: count of folded duplicates (always 1
  for the canonical case).

The frontend reading `primary_op_amount` from E02's response cannot
distinguish without checking `op_type`. 0167's "Issues Encountered"
already flags a possible rename `amount` → `appearance_count`; this
task either ships that rename (schema migration + ADR) or documents
the dual semantics in ADR 0037 / endpoint-queries README so consumers
don't misinterpret.

## Implementation Plan

### Step 1: investigate inner_tx_hash

- read `crates/xdr-parser/src/envelope.rs` and `transaction.rs` —
  does `extract_transactions` set `inner_tx_hash` for `TxFeeBump`
  envelopes?
- check the ingestion path in `crates/indexer/src/process.rs` and
  `staging.rs` — is the column written?
- audit existing data: `SELECT COUNT(*) FROM transactions WHERE
inner_tx_hash IS NULL AND <fee-bump heuristic>;` to gauge scope.
- decide: bug (fix + reindex) or expected (drop column / mark
  optional in ADR 0037).

### Step 2: decide on operations_appearances.amount

- Option A — rename to `appearance_count`. Schema migration, indexer
  update, all `oa.amount` SQL refs renamed (E02, E03, E07, E10, E13,
  E20, E22 + Rust handlers). New ADR documenting the rename.
- Option B — keep dual semantics, document. Add a note in ADR 0037
  §7 + endpoint-queries README. Update E02 header to say
  `primary_op_amount` is meaningful only for classic transfer op
  types; NULL it out in projection for soroban/appearance-only types.
- Owner picks.

### Step 3: ship the chosen fixes

- For Finding 1: indexer fix + targeted backfill of
  `inner_tx_hash` over existing partitions, OR a column-drop ADR if
  it's deliberate.
- For Finding 2: schema migration + reindex (Option A) OR doc-only
  PR (Option B).

## Acceptance Criteria

- [x] **Finding 1 resolved.** Indexer populates
      `transactions.inner_tx_hash` for fee-bump rows. Local reindex of
      audit ledger range (62016000–62016099) confirms 17 078 fee-bumps
      now have non-NULL `inner_tx_hash`; audit row `358ef42d…` carries
      `12021959…` byte-for-byte. Existing prod/staging partitions still
      need a reindex run for backfill — separate ops decision, not
      blocking the code fix.
- [x] **Finding 2 resolved.** Doc-only path chosen (team decision: schema
      with fold-count semantics stays; frontend list shows operation_count
      only). E02 primary-op preview cut; E03 `oa.amount` cut with `-- not
  in DB` marker pointing at archive XDR overlay (ADR 0029). README
      §03 entry updated.
- [x] Re-running E02 verification on the audit row no longer surfaces
      either discrepancy after local reindex: `primary_op_*` columns
      gone from response shape; `inner_tx_hash_hex` populated.
- [x] **Docs updated** — endpoint-queries README §03 amended; E02 / E03
      file headers + `Notes:` sections amended per
      [ADR 0032](../../2-adrs/0032_docs-architecture-evergreen-maintenance.md).
      ADR 0037 §7 left untouched — fold-count semantic was already
      documented in task 0163; no new schema shape introduced here.

## Implementation Notes

### F1 — `inner_tx_hash` (parser → indexer)

- **`crates/xdr-parser/src/envelope.rs`** — new public helper
  `inner_tx_hash(env, network_id) -> Option<[u8; 32]>` returning the
  inner-tx hash for `TxFeeBump` envelopes only (`None` for
  non-fee-bump). Refactored `tx_envelope_hash` to share a private
  `hash_tagged_transaction` so both helpers go through one
  SHA256-of-`TransactionSignaturePayload` pipeline.
- **`crates/xdr-parser/src/types.rs`** — added field
  `inner_tx_hash: Option<String>` to `ExtractedTransaction` (hex-encoded,
  same convention as `hash`).
- **`crates/xdr-parser/src/transaction.rs`** — `extract_single_transaction`
  now takes `network_id: &[u8; 32]`, calls the helper, populates the
  new field. Threading mirrors how `network_id` is already passed into
  `extract_envelopes`.
- **`crates/indexer/src/handler/process.rs`** — removed the empty
  `HashMap<String, Option<String>>` placeholder + the "follow-up parser
  work" comment that was the original gap marker.
- **`crates/indexer/src/handler/persist/{mod.rs,staging.rs}`** —
  `persist_ledger` / `Staged::prepare` no longer take an
  `inner_tx_hashes` parameter. `staging.rs:482` reads
  `tx.inner_tx_hash` directly off `ExtractedTransaction`.
- **`crates/indexer/tests/persist_integration.rs`** — 10
  `ExtractedTransaction` literals got `inner_tx_hash: None`; 8
  `let no_inner_tx_hashes = HashMap::new()` setups + 9
  `&no_inner_tx_hashes` arg lines stripped; unused `HashMap` import
  removed.

### F2 — fold-count `amount` exposure cut from API SQL

- **`02_get_transactions_list.sql`** — both Statement A and B: removed
  8 projected columns (`primary_op_type`, `primary_op_method`,
  `primary_op_from`, `primary_op_interacted_with`,
  `primary_op_interacted_with_kind`, `primary_op_amount`,
  `primary_op_asset_code`, `primary_op_asset_issuer`), the `pop`
  LATERAL, and 4 LEFT JOINs (`op_src`, `op_dst`, `op_ctr`, `op_iss`).
  `Notes:` section paragraph about StellarChain-style FROM/TO/AMOUNT
  preview replaced with a short rationale linking to 0163/0169.
- **`03_get_transactions_by_hash.sql`** — Statement C: `oa.amount`
  removed from per-op projection. Added `-- not in DB:` marker pointing
  at `envelope_xdr` / `result_meta_xdr` archive overlay (ADR 0029) for
  real per-op stroop amounts.
- **`README.md`** §03 — per-op field row no longer lists `amount`;
  added explanatory note that `operations_appearances.amount` is a
  fold count (task 0163), not a value.

### Tests

- 2 new unit tests in `envelope.rs`:
  - `inner_tx_hash_returns_none_for_non_fee_bump_envelopes` — V0 / V1 → None.
  - `inner_tx_hash_for_fee_bump_differs_from_outer_and_matches_inner_v1_hash` —
    inner hash equals standalone V1 hash of the same inner tx; differs
    from outer fee-bump hash.
- 1 new integration test in `tests/envelope_apply_order.rs`:
  - `inner_tx_hash_matches_stellar_core_for_every_fee_bump_in_audit_ledger` —
    iterates audit-batch ledgers, for every tx whose
    `TransactionResultResult` is `TxFeeBumpInner{Success,Failed}` it
    asserts `inner_tx_hash(env, mainnet_net_id) ==
InnerTransactionResultPair.transaction_hash` (the value
    stellar-core itself recorded in the meta during consensus).
    34/34 fee-bumps match byte-for-byte.

Pre-existing tests: 168 → 170 unit + 1 → 2 integration in xdr-parser;
9/9 indexer persist_integration; full workspace `cargo test` green.

## Design Decisions

### From Plan

1. **Fix Finding 1, not document-only.** lore-0168 already moved
   `source_id` into Horizon-canonical territory; leaving `inner_tx_hash`
   as a "follow-up parser work" TODO would be a half-done fee-bump
   story. Owner confirmed.

2. **Doc-only path for Finding 2** (vs. schema rename). Owner
   architectural decision: fold-count semantics on
   `operations_appearances.amount` stays (consistent across all three
   appearance tables); frontend list shows `operation_count` only.
   Schema rename across 3 partitioned tables + reindex was rejected as
   disproportionate to the naming-clarity gain.

### Emerged

3. **Helper signature returns `Option`, not panics on non-fee-bump.**
   `inner_tx_hash` could have been `pub fn ... -> [u8; 32]` panicking
   on V0/V1, mirroring how some XDR helpers do, but `Option` lets
   `extract_single_transaction` handle all envelope variants in a
   single match-free `.map(hex::encode)`. The unit test
   `inner_tx_hash_returns_none_for_non_fee_bump_envelopes` locks the
   contract.

4. **End-to-end protocol-conformance test against `InnerTransactionResultPair`,
   not Horizon.** Initial integration test compared against a hex
   string sourced via WebFetch from Horizon JSON. After owner pushback
   ("are you 10000% sure?"), rewrote to compare against
   `InnerTransactionResultPair.transaction_hash` extracted directly
   from the LedgerCloseMeta XDR — the canonical value stellar-core
   itself records during consensus. Stronger guarantee, no external
   service in the dependency chain.

5. **Drop the `inner_tx_hashes` HashMap entirely, don't keep it as
   optional.** The map was a legacy artifact of when the parser didn't
   compute the hash. With the field on `ExtractedTransaction` there is
   no second producer; keeping the parameter "just in case" would be a
   ghost knob in the persist API.

6. **F2 scope held to E02 + E03.** Did NOT extend cut to
   `soroban_events_appearances.amount` (E03 stmt E) or
   `soroban_invocations_appearances.amount` (E03 stmt F) despite same
   fold-count semantic problem. Owner scope was explicit on
   operations only; the events / invocations cut is a follow-up
   decision, not auto-spawned per the user-rule against
   spawning-without-asking.

## Issues Encountered

- **WebFetch hex-string fragility.** Initial Horizon-derived hex used
  in the integration test went through WebFetch's AI summarization. The
  string was correct (verified later via raw `curl` of Horizon JSON),
  but the trust path was weak. Replaced with direct
  `InnerTransactionResultPair.transaction_hash` extraction from
  LedgerCloseMeta — same dataset, no external service, 34 independent
  rows checked instead of 1.

- **`stellarchain.io` does not render `fee_account`.** During verification
  the user noted that stellarchain.io's tx-detail page does not show the
  fee-bump payer (`fee_account` in Horizon). Confirmed via raw Horizon
  JSON: `fee_account` IS present at the protocol level (in
  `FeeBumpTransaction.fee_source`); stellarchain.io chooses not to
  surface it. Our DB also doesn't carry `fee_account` — the
  `transactions.source_id` column post-0168 is always the inner source.
  Out of scope for 0169; flagged as potential future work.

- **Test fixture path coupled to specific Galexie batch name.** The
  integration test relies on
  `.temp/FC4DB5FF--62016000-62079999/FC4DB59C--62016099.xdr.zst` being
  present; absent the test silently skips (with eprintln). This is
  consistent with the existing alignment test from 0168 — same
  fallback. Not a regression.

## Future Work

These were spotted during the task but consciously NOT auto-spawned
per the user-rule "don't auto-spawn unless asked":

- **Backfill of `transactions.inner_tx_hash` on existing prod / staging
  partitions.** New ledgers ingested after this PR will have the
  column populated. Existing rows show NULL until a reindex (full or
  per-partition) runs. Owner / ops decision.

- **Cut fold-count `amount` exposure from E03 stmts E and F.**
  `soroban_events_appearances.amount` and
  `soroban_invocations_appearances.amount` carry the same
  fold-count-not-stroops semantics; same naming-confusion risk if
  surfaced via API. Held back per scope discipline.

- **Surface `fee_account` (fee-bump payer) in `transactions` schema.**
  Currently dropped at indexer level; only inner-source survives to
  DB. Some explorers surface it (stellar.expert "Fee payer"); ours
  could too if a frontend asks. Schema + indexer + ADR.

## Notes

- Audit row used as regression anchor:
  `358ef42d9840d91554a46d69be7c7fee8f8f4305379ab6ed614e4ea9ae4e75dc`
  on ledger `62016099` — fee-bump tx with consensus-recorded
  inner-tx hash `12021959a49f62ec43b6985a22682ca63104c4a99641a1f83e0986baf15b266d`.
  After this PR + reindex, the DB row mirrors that exactly.
- Local DB after reindex of audit range:
  68 MB / 100 ledgers / 36 319 tx (17 078 fee-bumps, 47%).
