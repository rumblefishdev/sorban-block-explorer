---
id: '0153'
title: 'BUG: wasm_hash FK violation on mid-stream backfill'
type: BUG
status: active
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

- [ ] `stub_unknown_wasm_interfaces` (or equivalent) runs before
      `upsert_contracts_returning_id` in `run_all_steps`.
- [ ] `upsert_wasm_metadata` overwrites stub metadata when a real
      upload is observed (ON CONFLICT DO UPDATE, not DO NOTHING).
- [ ] Integration test: synthetic ledger with a contract pointing at
      an unknown `wasm_hash` — persist succeeds, stub row exists,
      FK holds.
- [ ] Integration test: follow-up ledger brings the real WASM upload
      for the same `wasm_hash` — metadata updates from `{}` to real
      ABI.
- [ ] `backfill-bench --start 62016000 --end 62016999` (1000 ledgers)
      completes clean.
- [ ] `cargo clippy --all-targets -- -D warnings` green.
- [ ] `SQLX_OFFLINE=true cargo build --workspace` green.

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
