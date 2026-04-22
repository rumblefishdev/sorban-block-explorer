---
id: '0118'
title: 'BUG: NFT false positives from fungible token transfers'
type: BUG
status: active
related_adr: ['0027']
related_tasks: ['0026', '0027', '0149']
tags: [priority-high, effort-medium, layer-indexer, audit-F9]
milestone: 1
links:
  - crates/xdr-parser/src/nft.rs
  - lore/2-adrs/0027_post-surrogate-schema-and-endpoint-realizability.md
  - docs/audits/2026-04-10-pipeline-data-audit.md
history:
  - date: '2026-04-10'
    status: backlog
    who: stkrolikiewicz
    note: 'Spawned from pipeline audit finding F9 (HIGH severity).'
  - date: '2026-04-14'
    status: active
    who: fmazur
    note: 'Activated task for implementation.'
  - date: '2026-04-21'
    status: active
    who: stkrolikiewicz
    note: >
      Scope rewritten for post-0148 / post-ADR-0029 repo state. Task 0148
      removed `crates/db/src/soroban.rs` (incl.
      `update_contract_interfaces_by_wasm_hash()`) and trimmed
      `crates/indexer/src/handler/persist.rs` to a stub; task 0149 (Filip,
      in progress) is rebuilding `persist_ledger` against ADR 0027. The
      original implementation plan referenced functions / line numbers
      that no longer exist.

      New shape: three explicit phases — Phase 1 (parser-only WASM
      classification, startable immediately), Phase 2 (persist-time
      integration, gated on 0149 merge), Phase 3 (post-backfill cleanup,
      operational). Phase 1 delivers a testable `classify_contract_from_wasm_spec`
      function that Phase 2 drops into the write path once 0149 defines
      the new `persist_ledger` signature.
---

# BUG: NFT false positives from fungible token transfers

## Summary

`crates/xdr-parser/src/nft.rs` misclassifies SEP-0041 fungible token
transfers (USDC, XLM wrapping, etc.) as NFT events. The filter in
`looks_like_token_id()` accepts `i128` payloads, which are exactly the
standard fungible transfer amount type. At mainnet scale this would
flood the `nfts` table with millions of false-positive records.

The fix requires distinguishing NFT contracts from fungible contracts
by **WASM spec inspection**, not by payload-type heuristics — some NFT
contracts legitimately use `i128` as token IDs
(see `i128_token_id_not_excluded` test in `nft.rs`).

## Status: Active — phased

- **Phase 1 (parser)** — can start now, independent of other work.
- **Phase 2 (integration)** — gated on task 0149 merge (new
  `persist_ledger` signature).
- **Phase 3 (cleanup)** — operational, after production backfill.

## Context

SEP-0041 fungible token transfers emit events with the topic pattern
`["transfer", Address(from), Address(to)]` and `i128` amount as data —
identical in shape to SEP-0050 NFT transfer events that carry token IDs
as data. The current `nft.rs:162-174` filter exclusion list covers
`void`, `map`, `vec`, `error` but not numeric scalar types. The in-file
doc comment (2026-04-13 note) acknowledges the limitation and defers
the fix pending a proper spec-aware classifier.

ADR 0027 §7 `soroban_contracts` already carries a `contract_type
VARCHAR(50)` column; ADR 0027 §8 `wasm_interface_metadata.metadata JSONB`
contains the function signatures that let us classify. This task
connects the dots: derive classification from WASM spec, persist it on
`soroban_contracts.contract_type`, filter NFT inserts accordingly.

### Classification rules (OpenZeppelin `NonFungibleToken` vs `FungibleToken` traits)

Discriminators derived from the OpenZeppelin Stellar contracts
library (the de-facto reference linked from Stellar Developers docs):

- `packages/tokens/src/non_fungible/mod.rs` — `NonFungibleToken` trait.
- `packages/tokens/src/fungible/mod.rs` — `FungibleToken` trait (SEP-0041).

| Function              | NFT trait | Fungible trait | Discriminator? |
| --------------------- | :-------: | :------------: | :------------: |
| `owner_of`            |    yes    |       no       |      NFT       |
| `token_uri`           |    yes    |       no       |      NFT       |
| `approve_for_all`     |    yes    |       no       |      NFT       |
| `get_approved`        |    yes    |       no       |      NFT       |
| `is_approved_for_all` |    yes    |       no       |      NFT       |
| `decimals`            |    no     |      yes       |    Fungible    |
| `allowance`           |    no     |      yes       |    Fungible    |
| `total_supply`        |    no     |      yes       |    Fungible    |
| `balance`             |    yes    |      yes       |     shared     |
| `transfer`            |    yes    |      yes       |     shared     |
| `transfer_from`       |    yes    |      yes       |     shared     |
| `approve`             |    yes    |      yes       |     shared     |
| `name`, `symbol`      |    yes    |      yes       |     shared     |

- Any NFT discriminator present → classify as `Nft`.
- Otherwise, any Fungible discriminator present → `Fungible`.
- Dual-interface (both sets present) → `Nft` (safer: prefer false
  positives over false negatives for UX).
- No usable WASM metadata yet → `Other` (temporary until WASM upload
  is observed; see Phase 2 cache handling).
- SAC contracts (no WASM) → DB already labels them `'token'` at
  deploy time; Phase 2 treats `'token'` like `Fungible`.

Shared names — notably `balance` (returns `u32` for NFT count vs
`i128` for fungible amount) and `approve` (different signatures) —
are **not** discriminators. Name-level matching is sufficient for
Phase 1; signature-aware classification is a potential refinement
for a future enum variant.

## Implementation

### Phase 1 — parser classifier (no DB, no persist hook)

New public surface in `crates/xdr-parser`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractClassification {
    Nft,
    Fungible,
    Other,
}

pub fn classify_contract_from_wasm_spec(
    metadata: &serde_json::Value,
) -> ContractClassification;
```

Input shape: the `metadata` JSONB shape produced by
`extract_contract_interfaces` (`{functions: [{name, inputs, outputs},
...], wasm_byte_len: u64}`). Pure function — no I/O, no DB.

Unit tests (golden-style, fixtures in `tests/fixtures/`):

- Pure NFT contract (functions include `owner_of`, `token_uri`,
  `transfer`) → `Nft`.
- Pure fungible SEP-0041 contract (`decimals`, `allowance`,
  `transfer`) → `Fungible`.
- Dual-interface contract → `Nft` (precedence asserted).
- Empty / missing metadata → `Other`.
- Real-world mainnet fixtures: at least one known NFT contract
  (e.g., jamesbachini i128-id collection) and one known fungible
  (USDC SAC wrapper or canonical SEP-41 token).

Phase 1 does **not** modify `detect_nft_events` behavior — it only
adds the classifier function. `nft.rs:162-174` stays permissive.
Removing / updating `i128_token_id_not_excluded` test belongs to
Phase 2, when the actual filter decision shifts from heuristic to
classifier.

### Phase 2 — persist-time integration (gated on task 0149)

Once task 0149 merges and `persist_ledger` has a concrete signature
against the ADR 0027 schema:

1. **Classification persist path**: when `wasm_interface_metadata` is
   written, call `classify_contract_from_wasm_spec` and update
   `soroban_contracts.contract_type` in the same transaction (or as a
   follow-up UPDATE, depending on 0149's structure).
2. **Per-worker in-memory cache**:
   `HashMap<contract_id, ContractClassification>`, populated on demand.
   **Only cache definitive classifications** (`Nft`, `Fungible`,
   `Token` from SAC). Never cache `Other` — re-query DB on next
   encounter so a worker that saw a contract before its WASM was
   processed can pick up classification later.
3. **Batch cache population**: for each ledger, collect distinct
   `contract_id`s appearing in NFT-candidate events, issue one
   `SELECT contract_id, contract_type FROM soroban_contracts WHERE
contract_id = ANY($1)` to fill the cache in one round-trip before
   per-event filtering.
4. **Filter at NFT insert time**:
   - `Nft` → insert into `nfts`.
   - `Fungible` / `Token` → skip (no insert).
   - `Other` → insert (temporary false positive, cleaned up in
     Phase 3).
5. **Update test `i128_token_id_not_excluded`**: currently asserts the
   broken permissive behavior; rewrite to assert filter behavior per
   the classifier.

Exact function signatures, where the cache lives (per-request
struct? thread-local? worker state parameter?) depend on how task
0149 shapes `persist_ledger`. Finalise once that signature is public.

### Phase 3 — post-backfill cleanup (operational)

After the historical backfill run (task 0145) has processed the full
Soroban-era corpus:

```sql
BEGIN;
-- Sanity: how many unclassified contracts still have NFT rows?
SELECT COUNT(DISTINCT contract_id) AS unclassified
  FROM nfts
 WHERE contract_id IN (
     SELECT contract_id FROM soroban_contracts
      WHERE contract_type = 'other'
 );
-- If > 0: investigate unclassified contracts first.

-- Remove false positives from Phase 2 "Other" inserts.
DELETE FROM nfts
 WHERE contract_id IN (
     SELECT contract_id FROM soroban_contracts
      WHERE contract_type IN ('fungible', 'token')
 );
COMMIT;
VACUUM ANALYZE nfts;
```

Script committed to the repo (e.g.
`crates/db/migrations/` or a dedicated `ops/sql/` folder) so it is
reviewable and re-runnable.

## Acceptance Criteria

### Phase 1 (parser)

- [ ] `classify_contract_from_wasm_spec` function added to
      `crates/xdr-parser`, public surface.
- [ ] `ContractClassification` enum with `Nft` / `Fungible` / `Other`
      variants.
- [ ] Decision tree implemented per the classification-rules table
      above; dual-interface contracts classified as `Nft` (documented).
- [ ] Unit tests cover: pure NFT, pure fungible, dual-interface,
      empty metadata, and at least two real mainnet fixtures
      (one NFT with `i128` token_id, one SEP-41 fungible).
- [ ] No behavior change in `detect_nft_events` yet — Phase 1 only
      adds the classifier function.
- [ ] `nx run rust:build`, `nx run rust:test`, `nx run rust:lint`
      pass for the xdr-parser crate.

### Phase 2 (integration, gated on 0149)

- [ ] Classification writes `soroban_contracts.contract_type` on WASM
      upload processing.
- [ ] Per-worker cache avoids repeated DB lookups; does NOT cache
      `Other`.
- [ ] Batch cache population at ledger granularity (one query per
      ledger covering all candidate contracts).
- [ ] NFT insert path filters by classification: `Nft` → insert,
      `Fungible` / `Token` → skip, `Other` → insert (temporary).
- [ ] `i128_token_id_not_excluded` test rewritten to assert the new
      filter behavior.
- [ ] End-to-end test: live ingest of a small fixture range with one
      fungible contract + one NFT contract yields exactly the expected
      `nfts` rows (no USDC transfer leakage).

### Phase 3 (cleanup)

- [ ] SQL cleanup script committed to the repo; reviewable.
- [ ] Post-backfill dry run verifies sanity check returns 0
      unclassified-with-NFT-rows before the DELETE.
- [ ] `VACUUM ANALYZE nfts` in the operational runbook.

## Risks / Notes

- **Phase 2 signature dependency**: `persist_ledger` shape is being
  defined by task 0149; waiting on that merge avoids rework.
- **Parallel backfill races**: with multiple workers, a transfer event
  may arrive before the WASM upload of its contract is processed.
  Filter decision on `Other` deliberately inserts (false positive) and
  Phase 3 cleans up — simpler than synchronising workers.
- **Dual-interface precedence**: chose `Nft` over `Fungible` to prefer
  false positives over false negatives. Document this clearly so future
  readers don't change the precedence without understanding the
  rationale.
- **SAC contracts**: pre-classified as `'token'` at deploy time (no
  WASM); Phase 2 treats `'token'` identically to `'fungible'` in the
  filter.
- **Classifier evolution**: if new SEP specs introduce additional
  standard functions, extend the match table in Phase 1; adding new
  `ContractClassification` variants is additive and does not break
  Phase 2's filter (which treats unknown variants conservatively as
  `Nft`-insert — document this default explicitly when extending the
  enum).
