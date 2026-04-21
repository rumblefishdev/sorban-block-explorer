---
id: '0148'
title: 'Remove legacy write-path helpers; stub persist_ledger for ADR 0027 rewrite'
type: REFACTOR
status: completed
related_adr: ['0011', '0018', '0024', '0026', '0027']
related_tasks: ['0140']
tags: [layer-backend, layer-db, priority-high, effort-small, cleanup, adr-0027]
links:
  - crates/indexer/src/handler/persist.rs
  - crates/indexer/src/handler/convert.rs
  - crates/db/src/persistence.rs
  - crates/db/src/soroban.rs
history:
  - date: '2026-04-20'
    status: backlog
    who: fmazur
    note: >
      Created — task 0140 landed the ADR 0027 schema as fresh migrations, but
      the legacy write-path (persist_ledger chain, ~2343 loc across 4 files) still
      targets the pre-ADR shape and fails to compile against the new schema. Split
      off from 0140's follow-up surface: first delete the old write-path so the
      workspace turns green; a separate future task rebuilds persistence against
      ADR 0027.
  - date: '2026-04-20'
    status: backlog
    who: fmazur
    note: >
      Scope narrowed — delete only the helpers with no remaining callers after
      persist_ledger is stubbed (convert.rs, db::persistence, db::soroban).
      Keep persist_ledger itself with its current signature but empty body so
      process_ledger compiles unchanged; the follow-up task fills the body
      against the new schema.
  - date: '2026-04-20'
    status: active
    who: fmazur
    note: 'Activated task — promoted from backlog to active, set as current task.'
  - date: '2026-04-20'
    status: completed
    who: fmazur
    note: >
      Implemented — moved crates/db/src/{persistence,soroban}.rs and
      crates/indexer/src/handler/convert.rs to .trash/ (~1891 loc). Stubbed
      persist_ledger body (signature unchanged). Dropped dangling module
      declarations in db/src/lib.rs and handler/mod.rs. Pruned crates/db/
      Cargo.toml (unused domain dep + dev-deps removed, serde_json
      feature-gated). Workspace green — cargo check, cargo clippy
      --all-targets -- -D warnings, cargo build --workspace, npm run db:reset,
      npm run db:prepare all pass. Pre-push hook passes without --no-verify.
---

# Remove legacy write-path helpers; stub persist_ledger for ADR 0027 rewrite

## Summary

Clear out the pre-ADR-0027 write-path without touching the parsing pipeline.
Delete only what will have zero callers after the change: the three helper
modules (`crates/db/src/persistence.rs`, `crates/db/src/soroban.rs`,
`crates/indexer/src/handler/convert.rs`). Keep `persist_ledger` in place with
its current signature but an empty body — `process_ledger` still calls it,
and the follow-up task fills in the body against the new schema.

No production data to preserve. After this task, indexer parses ledgers
end-to-end but writes nothing to the DB.

## Context

Task 0140 rewrote the schema and the `domain/` read-path from scratch to
match ADR 0027. The write-path was deliberately out of scope: PR #98 ships
with `cargo check` red below `crates/domain` because `persist_ledger` and its
callees still reference pre-ADR columns, `VARCHAR` account keys, `String`
hashes, and `JSONB` blobs that no longer exist.

The legacy write-path has two layers:

| Layer                  | Files                                   |  LOC | Caller surface                            |
| ---------------------- | --------------------------------------- | ---: | ----------------------------------------- |
| Orchestrator           | `crates/indexer/src/handler/persist.rs` |  452 | `process.rs::process_ledger` (13 args)    |
| Helpers (to be purged) | `crates/indexer/src/handler/convert.rs` |  183 | Only `persist::persist_ledger` uses these |
|                        | `crates/db/src/persistence.rs`          |  498 | Only `persist::persist_ledger` uses these |
|                        | `crates/db/src/soroban.rs`              | 1210 | Only `persist::persist_ledger` uses these |

Once `persist_ledger`'s body is empty, the three helper files have zero
callers, so deleting them is safe. `persist_ledger` itself stays — its
signature pins the contract `process_ledger` depends on, and the follow-up
task will wire the new schema inside it.

## Implementation Plan

### Step 1: Move helper files to `.trash/`

Project policy: use `mv`, not `rm`. Embedded `#[cfg(test)]` modules go with them.

```
crates/db/src/persistence.rs                → .trash/legacy-write-path-pre-adr-0027/
crates/db/src/soroban.rs                    → .trash/legacy-write-path-pre-adr-0027/
crates/indexer/src/handler/convert.rs       → .trash/legacy-write-path-pre-adr-0027/
```

### Step 2: Drop the now-dangling module declarations

- `crates/db/src/lib.rs` — remove `pub mod persistence;` and `pub mod soroban;`
- `crates/indexer/src/handler/mod.rs` — remove `mod convert;` (leave `mod persist;`)

### Step 3: Stub `persist_ledger`

`crates/indexer/src/handler/persist.rs` — keep the file and the function, but:

- Drop `use db::persistence::…`, `use db::soroban::…`, `use super::convert`.
- Drop imports of `xdr_parser::types::Extracted*` that become unused.
- Keep `persist_ledger`'s full signature (13 arguments) — `process_ledger` calls
  it verbatim.
- Replace the body with a stub that silences unused warnings and returns `Ok(())`:

```rust
pub async fn persist_ledger(
    db_tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    ledger: &ExtractedLedger,
    transactions: &[ExtractedTransaction],
    operations: &[(String, Vec<ExtractedOperation>)],
    events: &[(String, Vec<ExtractedEvent>)],
    invocations: &[(String, Vec<ExtractedInvocation>)],
    operation_trees: &[(String, serde_json::Value)],
    contract_interfaces: &[ExtractedContractInterface],
    contract_deployments: &[ExtractedContractDeployment],
    account_states: &[ExtractedAccountState],
    liquidity_pools: &[ExtractedLiquidityPool],
    pool_snapshots: &[ExtractedLiquidityPoolSnapshot],
    tokens: &[ExtractedToken],
    nfts: &[ExtractedNft],
) -> Result<(), HandlerError> {
    // TODO(adr-0027-writes): wire new write-path against the ADR 0027 schema.
    // Body intentionally empty — indexer parses but does not persist until the
    // follow-up task replaces this stub.
    let _ = (
        db_tx, ledger, transactions, operations, events, invocations,
        operation_trees, contract_interfaces, contract_deployments,
        account_states, liquidity_pools, pool_snapshots, tokens, nfts,
    );
    Ok(())
}
```

### Step 4: Verify workspace turns green

```
npm run db:reset
cargo check --workspace
cargo clippy --all-targets -- -D warnings
SQLX_OFFLINE=true cargo build --workspace
npm run db:prepare        # no query!() callsites left that target removed tables
```

Commit the regenerated `.sqlx/` offline cache.

## Acceptance Criteria

- [x] Three helper files moved to `.trash/legacy-write-path-pre-adr-0027/`
- [x] `crates/db/src/lib.rs` no longer exports `persistence` or `soroban`
- [x] `crates/indexer/src/handler/mod.rs` no longer declares `convert`
      (still declares `persist`)
- [x] `persist_ledger` keeps its signature; body is a stub returning `Ok(())`
- [x] `process_ledger` and `backfill-bench` compile unchanged
- [x] `cargo check --workspace` green
- [x] `cargo clippy --all-targets -- -D warnings` green
- [x] `npm run db:prepare` succeeds; `.sqlx/` not needed (no `sqlx::query!()`
      macros remain after helper removal — only runtime `sqlx::query()` calls)
- [x] Pre-push hook passes without `--no-verify`

## Implementation Notes

- Moved to `.trash/legacy-write-path-pre-adr-0027/`:
  - `crates/db/src/persistence.rs` (498 loc)
  - `crates/db/src/soroban.rs` (1210 loc)
  - `crates/indexer/src/handler/convert.rs` (183 loc)
- `crates/indexer/src/handler/persist.rs` — body emptied; signature (14 params)
  unchanged so `process_ledger` and `backfill-bench` still compile. Unused
  `use db::persistence::…`, `use db::soroban::…`, and `use super::convert`
  dropped; only `xdr_parser::types::Extracted*` + `super::HandlerError` remain.
- `crates/db/src/lib.rs` — removed `pub mod persistence;` and `pub mod soroban;`.
- `crates/indexer/src/handler/mod.rs` — removed `mod convert;`; `mod persist;`
  retained.
- `crates/db/Cargo.toml` — pruned to match the new surface: dropped
  `domain = { path = "../domain" }` (unused), dropped the entire
  `[dev-dependencies]` block (the `#[cfg(test)]` modules lived in the removed
  files), and made `serde_json` optional via the `aws-secrets` feature (only
  `secrets.rs` uses it).

## Design Decisions

### From Plan

1. **Keep `persist_ledger`, delete helpers.** Only the three helper modules
   have zero callers after the stub; `persist_ledger` itself stays because
   `process_ledger` and `backfill-bench` still call it. Follow-up task fills
   the body against the ADR 0027 schema.
2. **Empty body over `unimplemented!()`.** `unimplemented!()` would panic at
   runtime — that would break the indexer Lambda even though parsing is fine.
   Returning `Ok(())` lets the pipeline parse and log end-to-end, just with
   no DB writes.

### Emerged

3. **Pruned `crates/db/Cargo.toml` unused deps.** Not in the original plan.
   After removing `persistence.rs` / `soroban.rs`, the `db` crate no longer
   uses `domain` or `serde_json` outside the `aws-secrets` feature, and
   `[dev-dependencies]` only existed for the `#[cfg(test)]` modules that went
   to `.trash/`. rust-analyzer was surfacing the dangling deps as errors on
   `lib.rs`. Cleanup: dropped `domain`, gated `serde_json` behind
   `aws-secrets`, removed `[dev-dependencies]`.

## Issues Encountered

- **`npm run db:prepare` emits `warning: no queries found`.** Not an error —
  after removing the helper modules, the workspace has zero `sqlx::query!()`
  macro callsites. Only runtime `sqlx::query()` remains (in
  `db-partition-mgmt` and `backfill-bench::ledger_exists`), and runtime
  queries don't use the offline cache. `.sqlx/` does not exist in the repo;
  no commit needed. The acceptance criterion was adjusted to reflect this.

## Out of Scope

- Filling in `persist_ledger` against the ADR 0027 schema (new `insert_*` /
  `upsert_*` implementations, StrKey → `accounts.id` resolver, BYTEA hash
  binding). **This is the next follow-up** — it replaces the stub body in
  `persist.rs` and may rebuild converters inline or as a new helper module.
- API layer JOIN updates (ADR 0027 Part III endpoints).
- Partition Lambda update to cover all 8 partitioned tables and switch
  `operations` to `created_at` partitioning.

## Notes

- `db::pool`, `db::migrate`, `db::secrets` stay — schema-agnostic, used by
  `db-migrate`, `db-partition-mgmt`, `indexer`, `backfill-bench`.
- Indexer Lambda keeps parsing end-to-end; it just writes nothing. The
  CloudWatch `LastProcessedLedgerSequence` metric still publishes because
  `process_ledger`'s post-commit block is untouched.
- `backfill-bench::ledger_exists` still reads `ledgers.sequence` (unchanged
  by ADR 0027), so partition-skip logic stays correct even though no rows
  land.
- This task unblocks CI: PR #98 currently pushes with `--no-verify`; after
  this lands, the pre-push `cargo clippy` hook should pass.
