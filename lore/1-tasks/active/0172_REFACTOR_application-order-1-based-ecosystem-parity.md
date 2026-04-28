---
id: '0172'
title: 'REFACTOR: switch application_order from 0-based to 1-based for Stellar ecosystem parity'
type: REFACTOR
status: active
related_adr: ['0028', '0037']
related_tasks: ['0167', '0168', '0169']
tags:
  [
    indexer,
    schema,
    api,
    application-order,
    ecosystem-parity,
    pre-mainnet-backfill,
  ]
links:
  - 'crates/indexer/src/handler/persist/staging.rs'
  - 'crates/api/src/stellar_archive/dto.rs'
  - 'lore/2-adrs/0028_parsed-ledger-artifact-v1-shape.md'
  - 'docs/architecture/database-schema/endpoint-queries/02_get_transactions_list.sql'
history:
  - date: 2026-04-28
    status: backlog
    who: fmazur
    note: >
      Spawned from manual E02 verification. 8/8 sampled transactions
      across 5 ledgers confirm DB application_order is systematically
      0-based while Horizon paging_token (and the rest of the Stellar
      ecosystem — stellar.expert, stellar-core) is 1-based. Off-by-1
      is structural, not a data corruption: `staging.rs:479` uses
      Rust's `.enumerate()` (0-indexed) and ADR 0028 documents the
      0-based convention as a deliberate choice. Owner has not yet
      started mainnet backfill, so reindex cost is contained — fix
      the convention now before backfill locks data shape.
  - date: 2026-04-28
    status: active
    who: fmazur
    note: 'Promoted to active via /promote-task'
---

# REFACTOR: switch application_order from 0-based to 1-based for Stellar ecosystem parity

## Summary

`transactions.application_order` and `operations_appearances.application_order`
are stored 0-based by the indexer. The rest of the Stellar ecosystem
(Horizon `paging_token`, stellar-core history records, stellar.expert,
SDK) uses 1-based application_order. Switch to 1-based at the indexer
boundary, update ADR 0028, and reindex. Mainnet backfill has not yet
started — this is the cheapest moment to make the change.

## Context

### How the divergence was found

During manual verification of `GET /transactions` (E02) against Horizon
mainnet, the response field `application_order` matched Horizon by
**relative ordering** but was **systematically off by 1 in absolute
value**. Verification:

| Sample                                       | DB app_order | Horizon (decoded from paging_token) | Diff |
| -------------------------------------------- | ------------ | ----------------------------------- | ---- |
| ledger 62016000, first tx (ASC)              | 0            | 1                                   | +1   |
| ledger 62016099, first tx (ASC)              | 0            | 1                                   | +1   |
| ledger 62016099, last tx (DESC, of 256)      | 255          | 256                                 | +1   |
| ledger 62016099, app_order=255 (penultimate) | 254          | 255                                 | +1   |
| ledger 62016099, app_order=247               | 246          | 247                                 | +1   |
| ledger 62016010, app_order=50                | 49           | 50                                  | +1   |
| ledger 62016050, app_order=100               | 99           | 100                                 | +1   |
| ledger 62016080, first tx (ASC)              | 0            | 1                                   | +1   |

8/8 samples consistent. Ledger 62016099 has `transaction_count = 256`
in both DB and Horizon — DB stores `[0, 255]`, Horizon represents `[1, 256]`.
`extra_bits = 0` on every paging_token decode, ruling out a different
formula.

### Where the 0-based convention comes from

```rust
// crates/indexer/src/handler/persist/staging.rs:479
for (app_order, tx) in transactions.iter().enumerate() {
    // ...
    application_order: app_order.try_into()...
}
```

Rust's `.enumerate()` is 0-indexed. The choice was codified in:

- **ADR 0028** §parsed_ledger.json: _"`application_order` — 0-based tx
  index within the ledger."_
- **DTO comment** `crates/api/src/stellar_archive/dto.rs:100`:
  _"Application order within the transaction (zero-based)."_
- **Audit doc** `docs/database-audit-first-implementation.md:133`: _"Zero-based
  index of this operation within its parent transaction."_

So the divergence is by design — but the design was wrong. Stellar
protocol's deterministic tx ordering is consumed by Horizon as 1-based
in `paging_token` (formula `(ledger << 32) | (tx_app_order << 12)`)
and rendered as 1-based by every public explorer. A user side-by-siding
our explorer with stellar.expert or Horizon will see different numbers
for the same hash — silent UX bug.

### Why fix now

Mainnet backfill has not started. The current DB holds ~100 audit ledgers
(62016000–62016099, ~3300 tx). Reindex cost is trivial today; once backfill
runs against ~50M+ ledgers, fixing means a multi-day rebuild.

## Implementation Plan

### Step 1: Indexer — switch to 1-based

`crates/indexer/src/handler/persist/staging.rs` line 479:

```rust
// Before
for (app_order, tx) in transactions.iter().enumerate() {

// After
for (idx, tx) in transactions.iter().enumerate() {
    let app_order = idx + 1;  // 1-based to match Stellar ecosystem (Horizon paging_token)
```

Same pattern applies wherever `operations_appearances.application_order`
is assigned by enumeration over a tx's operations vec. Audit:

```bash
grep -rn "enumerate" crates/indexer crates/xdr-parser | grep -i "order\|appearance"
```

Any `(i, _)` pattern feeding an `application_order` field needs `+1`.

### Step 2: Update ADR 0028

Edit `lore/2-adrs/0028_parsed-ledger-artifact-v1-shape.md` line 267:

```diff
- `application_order` — 0-based tx index within the ledger.
+ `application_order` — 1-based tx index within the ledger
+   (matches Horizon `paging_token` decoding and stellar-core history
+   record convention; bumped from 0-based per task 0172).
```

Add a `## Decision Update` block with date + task ref.

### Step 3: Update DTO comments

`crates/api/src/stellar_archive/dto.rs:100`:

```diff
- /// Application order within the transaction (zero-based).
+ /// Application order within the transaction (1-based, matches
+ /// Horizon paging_token convention).
```

`docs/database-audit-first-implementation.md:133` — same flip.

### Step 4: Schema — optional CHECK constraint

Currently `application_order SMALLINT NOT NULL`. Optionally add a CHECK
to enforce ≥ 1 going forward:

```sql
ALTER TABLE transactions
  ADD CONSTRAINT chk_tx_app_order_positive CHECK (application_order >= 1);
```

Same for `operations_appearances.application_order`. **Defer if owner
prefers no constraint** — the indexer fix alone is sufficient and
constraints add migration overhead.

### Step 5: Tests

Existing unit tests on `staging.rs` may assert 0-based values. Update
expected values to 1-based. New unit test:

```rust
#[test]
fn application_order_is_one_based() {
    // Build a 3-tx fixture, run staging, assert app_orders == [1, 2, 3]
}
```

Plus integration: re-run the audit ledger 62016099 fixture and assert
`MIN(application_order) = 1 AND MAX(application_order) = 256`.

### Step 6: Reindex

Per task 0168 reindex strategy (full backfill option):

1. Truncate ledger-derived tables (transactions, operations_appearances,
   transaction_participants, soroban_events_appearances,
   soroban_invocations_appearances, etc.).
2. Replay from Galexie / parsed_ledger blobs through the fixed indexer.
3. Verify post-reindex: `MIN(application_order) = 1` on `transactions`
   and `operations_appearances`.

### Step 7: Verification against Horizon

Sample 20 random tx, decode Horizon paging_token, assert
`db.application_order == (paging_token - ledger × 2^32) / 4096`.
Script lives in `docs/architecture/database-schema/endpoint-queries/`
or `scripts/`. 0/20 mismatches is the bar.

## Acceptance Criteria

- [ ] `crates/indexer/src/handler/persist/staging.rs` (and any sibling
      enumerations writing `application_order`) emit 1-based values.
- [ ] ADR 0028 updated to document 1-based convention with task 0172
      decision-update note.
- [ ] DTO and audit-doc comments updated to "1-based".
- [ ] (Optional) CHECK constraint `application_order >= 1` added on
      `transactions` and `operations_appearances`.
- [ ] Unit test asserting 1-based sequence in staging output.
- [ ] Integration test on audit ledger fixture: `MIN = 1, MAX = transaction_count`.
- [ ] Reindex executed; post-reindex spot-check on 20 random tx vs
      Horizon paging_token: 0 mismatches.
- [ ] **Docs updated** — per ADR 0032: - [ ] `lore/2-adrs/0028_parsed-ledger-artifact-v1-shape.md` — updated. - [ ] `docs/architecture/database-schema/endpoint-queries/README.md`
      §02 — note convention if helpful (or N/A — convention now
      matches ecosystem so no special note needed). - [ ] ADR 0037 schema snapshot — N/A (column type unchanged; only
      value semantics).

## Notes

- **Why REFACTOR not BUG:** the current behavior is documented in ADR 0028
  as a deliberate choice. We are reversing the choice, not fixing a
  parser miscompute. It does cause downstream UX divergence, hence
  worth doing — but the type is REFACTOR.
- **Out of scope:**
  - Changing how cursor pagination encodes tx position (cursor uses
    `(created_at, id)` pair, not `application_order`; unaffected).
  - Adding a Horizon-style `paging_token` field to API responses
    (separate consideration; not requested).
- **Co-located concerns the owner should consider:**
  - If the API ever exposes an "operation_application_order" within a
    tx, that one is also currently 0-based per the same `.enumerate()`
    pattern in operations extraction. Worth flipping in the same PR
    for consistency. Audit before committing.
