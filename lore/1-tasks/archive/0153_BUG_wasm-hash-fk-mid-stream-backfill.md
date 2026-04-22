---
id: '0153'
title: 'BUG: wasm_hash FK violation on mid-stream backfill'
type: BUG
status: completed
related_adr: ['0030']
related_tasks: ['0151', '0152']
tags:
  [layer-indexer, layer-db, priority-high, effort-small, bug, schema, backfill]
links:
  - crates/indexer/src/handler/persist/write.rs
  - crates/indexer/src/handler/persist/staging.rs
  - crates/db/migrations/0002_identity_and_ledgers.sql
history:
  - date: '2026-04-22'
    status: backlog
    who: fmazur
    note: >
      Spawned from 0152 future work. Found during a 1000-ledger backfill
      bench (62016000-62016999): at ledger 62016744, an INSERT into
      soroban_contracts violates soroban_contracts_wasm_hash_fkey because
      the referenced wasm_hash was uploaded in a pre-62016000 ledger
      (outside the backfill window). The FK has been in place since
      ADR 0030 / task 0151; the 100-ledger bench never crossed this
      boundary because partition 62016000 is self-contained for
      contracts deployed within it. Bug is unrelated to ADR 0031.
  - date: '2026-04-22'
    status: active
    who: fmazur
    note: >
      Promoted to active right after landing 0152. Will tackle stub-row
      fix (Option 1 from task description) to unblock 1000+ ledger bench.
  - date: '2026-04-22'
    status: completed
    who: fmazur
    note: >
      Landed. Option 1 stub path implemented. 3 files touched: write.rs
      (new stub_unknown_wasm_interfaces), persist/mod.rs (pipeline wire
      under wasm_ms timing), persist_integration.rs (new 2-ledger test
      covering unknown-hash insert + real-upload upgrade). All 7
      acceptance criteria met. 1000-ledger bench 62016000-62016999
      completes clean (mean 252 ms, p95 359 ms — in range of 0151/0152
      baselines 309-384 ms, no regression). wasm_ms consistently 0 in
      bench logs — stub UNNEST+ON CONFLICT DO NOTHING is free in
      practice. Out-of-Scope items (fetch-on-read ADR candidate,
      misnamed wasm_uploaded_at_ledger field) left as prose — no
      follow-up tasks spawned per owner request.
---

# BUG: wasm_hash FK violation on mid-stream backfill

## Summary

`soroban_contracts.wasm_hash` has a `REFERENCES wasm_interface_metadata(wasm_hash)`
FK (from ADR 0030). On mid-stream backfill, contracts deployed in-window
can reference WASMs uploaded before the window — those WASMs don't exist
locally, so the contract INSERT fails with FK violation and the whole
ledger errors out.

## Context

Reproduction:

```
DATABASE_URL=... cargo run --release -p backfill-bench -- \
  --start 62016000 --end 62016999
```

Crashes at ledger 62016744 with:

```
Error: Database(PgDatabaseError {
  message: "insert or update on table \"soroban_contracts\" violates
            foreign key constraint \"soroban_contracts_wasm_hash_fkey\"",
  detail: "Key (wasm_hash)=(\\xf340…9783) is not present in table
           \"wasm_interface_metadata\".",
  constraint: "soroban_contracts_wasm_hash_fkey",
})
```

Root cause: contract deployed @ 62016744 references a WASM uploaded at
some ledger before 62016000, so the WASM row is missing locally.

Why didn't 0152's 100-ledger bench hit this: partition 62016000 happens
to be self-contained — all WASMs referenced by contracts deployed in
62016000..62016099 were also uploaded in that range. Wider windows
(1000+ ledgers or earlier backfill start points) surface the bug.

Blocks: any bench / production backfill that starts mid-chain. At
mainnet scale this is the default (no one backfills from genesis).

## Implementation

### Option 1 — stub `wasm_interface_metadata` row (recommended)

Before the `upsert_contracts_returning_id` pass that inserts rows with
`wasm_hash`, pre-insert stub rows for every `wasm_hash` referenced in
`staged.contract_rows` that isn't already in `staged.wasm_rows`:

```rust
// In write.rs, add before upsert_contracts_returning_id:
async fn stub_unknown_wasm_interfaces(
    db_tx: &mut Transaction<'_, Postgres>,
    staged: &Staged,
) -> Result<(), HandlerError> {
    let staged_hashes: HashSet<[u8; 32]> =
        staged.wasm_rows.iter().map(|r| r.wasm_hash).collect();
    let needed: Vec<Vec<u8>> = staged
        .contract_rows
        .iter()
        .filter_map(|r| r.wasm_hash)
        .filter(|h| !staged_hashes.contains(h))
        .map(|h| h.to_vec())
        .collect();
    if needed.is_empty() {
        return Ok(());
    }
    sqlx::query(
        r#"
        INSERT INTO wasm_interface_metadata (wasm_hash, metadata)
        SELECT wh, '{}'::jsonb
          FROM UNNEST($1::BYTEA[]) AS t(wh)
        ON CONFLICT (wasm_hash) DO NOTHING
        "#,
    )
    .bind(&needed)
    .execute(&mut **db_tx)
    .await?;
    Ok(())
}
```

Also verify `upsert_wasm_metadata` uses `ON CONFLICT DO UPDATE SET
metadata = EXCLUDED.metadata` (or COALESCE with preference for
non-stub) so when the real upload is later observed, the stub is
upgraded in place.

### Acceptance Criteria

- [x] `stub_unknown_wasm_interfaces` (or equivalent) runs before
      `upsert_contracts_returning_id` in `run_all_steps`.
- [x] `upsert_wasm_metadata` overwrites stub metadata when a real
      upload is observed (ON CONFLICT DO UPDATE, not DO NOTHING).
- [x] Integration test: synthetic ledger with a contract pointing at
      an unknown `wasm_hash` — persist succeeds, stub row exists,
      FK holds.
- [x] Integration test: follow-up ledger brings the real WASM upload
      for the same `wasm_hash` — metadata updates from `{}` to real
      ABI.
- [x] `backfill-bench --start 62016000 --end 62016999` (1000 ledgers)
      completes clean.
- [x] `cargo clippy --all-targets -- -D warnings` green.
- [x] `SQLX_OFFLINE=true cargo build --workspace` green.

## Implementation Notes

Files changed (3):

- `crates/indexer/src/handler/persist/write.rs` — added
  `pub(super) async fn stub_unknown_wasm_interfaces`. Builds a
  `HashSet<[u8; 32]>` from `staged.wasm_rows`, walks `staged.contract_rows`
  to collect each referenced `wasm_hash` not already in the set
  (with a local `seen: HashSet` to dedupe across contract rows), then
  emits one UNNEST `INSERT … ON CONFLICT DO NOTHING` with
  `metadata = '{}'::jsonb`. Also added `HashSet` to the `std::collections`
  import line.
- `crates/indexer/src/handler/persist/mod.rs` — one-line wire inside
  `run_all_steps`: `write::stub_unknown_wasm_interfaces(db_tx, staged).await?;`
  sits between `upsert_wasm_metadata` and `upsert_contracts_returning_id`,
  both inside the same `wasm_ms` timing window (no new StepTimings field).
- `crates/indexer/tests/persist_integration.rs` — new test
  `stub_wasm_unblocks_unknown_hash_and_real_upload_upgrades_it` using
  dedicated fixtures (STUB_LEDGER_SEQ, STUB_CONTRACT, STUB_WASM_HASH).
  Two-ledger flow:
  1. Ledger with contract deployment carrying an unknown `wasm_hash`,
     empty `contract_interfaces` — asserts persist succeeds, stub row
     exists with `metadata = {}`, `soroban_contracts.wasm_hash` is set.
  2. Follow-up ledger with `contract_interface` for the same
     `wasm_hash` — asserts `wasm_interface_metadata.metadata` is
     overwritten to the real ABI JSON (`{functions: [], wasm_byte_len: 512}`).
     Helper `clean_stub_test` wipes leaves in FK-safe order.

Bench (acceptance #5): 1000 ledgers 62016000-62016999 completed
clean. Stats: min 79 ms / mean 252 ms / p50 253 ms / p95 359 ms /
p99 416 ms / max 612 ms. `wasm_ms = 0` consistently across all
1000 ledgers — stub INSERT is free in practice (UNNEST of a few
BYTEA hashes with ON CONFLICT DO NOTHING). Comparable baselines:
0149 post-diag-filter p95 309 ms, 0151 ADR-0030 p95 309 ms, 0152
ADR-0031 p95 384 ms. No regression.

## Design Decisions

### From Plan

1. **Option 1 (stub) over alternatives.** Task description listed
   Option 1 as recommended. Alternatives considered implicitly:
   (a) drop the FK — loses referential integrity, makes orphan-hash
   bugs silent; (b) DEFERRABLE FK — doesn't help mid-stream because
   the WASM was never inserted, not just late; (c) fetch-on-read
   (0154 follow-up) — architectural, out of scope for an unblocker.

2. **Stub INSERT uses `ON CONFLICT DO NOTHING`, not `DO UPDATE`.**
   A replay of a stubbed ledger must NOT overwrite real metadata
   that arrived in a later ledger. The upgrade path is the other
   direction: `upsert_wasm_metadata` uses `DO UPDATE SET metadata =
EXCLUDED.metadata` (already from 0149), so when the real upload
   lands it overwrites the stub.

### Emerged

3. **Stub call bundled under `wasm_ms` timing, not a new
   `stub_wasm_ms` field.** Plan didn't specify. Given the measured
   cost is ≤0 ms (consistently `wasm_ms=0` in bench logs), adding
   another StepTimings field would be noise. Left as part of the
   "wasm group" step alongside `upsert_wasm_metadata`.

4. **Local `seen: HashSet<[u8; 32]>` in the collect loop.** Task
   sketch used `.filter(|h| !vec.contains(h))` which is O(n²) if
   many contracts in one ledger share a wasm_hash. Swapped for an
   explicit loop with a dedupe set. Zero behaviour change, better
   worst-case. Still trivially small in practice (handful of
   deployments per ledger).

5. **No `COALESCE` / stub-aware branch in `upsert_wasm_metadata`.**
   The task sketch floated `COALESCE with preference for non-stub`
   as an option. Skipped because the parser only stages a WasmRow
   when it has a real `ExtractedContractInterface` — there is no
   code path that could feed `{}` back through `upsert_wasm_metadata`
   and clobber real data. The plain `DO UPDATE SET metadata =
EXCLUDED.metadata` is safe.

## Issues Encountered

- **Missing `Value` import in the integration test.** Added
  `let stub_metadata: Value = …` but `Value` wasn't in scope —
  test module only imported `serde_json::json`. Widened to
  `use serde_json::{Value, json};`. One-shot fix, caught by the
  first `cargo build` attempt.
- **`ContractType::Unknown` doesn't exist.** First test draft used
  `ContractType::Unknown` as a neutral contract_type for the stub
  fixture. Current enum has only `Token` (0) and `Other` (1) per
  ADR 0031. Changed fixture to `ContractType::Other`. Not a
  regression — the sketch was just wrong about the enum shape.

## Future Work

Kept as prose (not spawned to backlog, per owner request):

- ADR candidate: fetch-on-read vs. cached `wasm_interface_metadata`.
  Precedent in ADR 0029 (parsed-artifact abandon). Stub fix coexists
  with either outcome.
- Cleanup: rename / correctly source
  `soroban_contracts.wasm_uploaded_at_ledger`, which today is
  populated from `dep.deployed_at_ledger` (not the actual upload
  ledger of the WASM).

## Out of Scope

- Full architectural question "do we need `wasm_interface_metadata` at
  all" (candidate follow-up ADR — fetch WASM from S3 read-time vs.
  keep precomputed cache). Stub fix unblocks backfill regardless of
  that decision.
- Fixing the misnamed `wasm_uploaded_at_ledger` field in
  `soroban_contracts` (currently populated from `dep.deployed_at_ledger`,
  not the actual upload ledger) — separate cleanup task.

## Notes

- Stub metadata is safe because WASM bytes are cryptographically
  identified by hash. Once the real upload is observed (in the same
  or future backfill), the extracted ABI is deterministic — overwriting
  `{}` with real ABI is always correct. See 0152 discussion.
- Precedent for "fetch on read" architecture exists in ADR 0029
  (abandoned parsed artifacts for tx details). If someone drafts an
  ADR to drop `wasm_interface_metadata` entirely, this fix can coexist
  as an interim unblock.
