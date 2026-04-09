---
id: '0113'
title: 'BUG: indexer parse_s3_key rejects all Galexie files — staging DB empty'
type: BUG
status: active
related_adr: []
related_tasks: ['0034', '0108']
tags: [bug, indexer, xdr-parser, staging, priority-high, effort-small]
milestone: 1
links:
  - crates/xdr-parser/src/lib.rs
  - crates/indexer/src/handler/mod.rs
  - infra/src/lib/stacks/compute-stack.ts
history:
  - date: '2026-04-08'
    status: backlog
    who: stkrolikiewicz
    note: 'Bug discovered while debugging "files in bucket but no DB records" on staging. Two interacting issues: parse_s3_key expects wrong file extension and format, and tracing is silently filtering all output.'
  - date: '2026-04-08'
    status: active
    who: stkrolikiewicz
    note: 'Activated — priority-high, blocks staging indexer functionality.'
  - date: '2026-04-08'
    status: backlog
    who: stkrolikiewicz
    note: 'Unactivated — deferred, returning to backlog.'
  - date: '2026-04-09'
    status: active
    who: fmazur
    note: 'Activated — Galexie running on staging, indexer pipeline broken'
---

# BUG: indexer parse_s3_key rejects all Galexie files

## Symptom

- Galexie writes ledger files to `s3://staging-stellar-ledger-data/` continuously (~12 files/min, files visible via `aws s3 ls`).
- Indexer Lambda **is invoked** by S3 PutObject events: 2076 invocations in 2 hours, **0 errors**, sub-1 ms duration each.
- **DB has zero records** in `ledgers` / `transactions` / etc.
- No application logs appear in CloudWatch beyond Lambda runtime boilerplate (`INIT_START`, `START`, `END`, `REPORT`, `XRAY`).

## Root cause — two bugs interacting

### Bug 1 — `parse_s3_key` rejects every Galexie file (silent skip)

**File:** `crates/xdr-parser/src/lib.rs:87-127`

`parse_s3_key` expects file extension **`.xdr.zstd`** (with `d`):

```rust
.strip_suffix(".xdr.zstd")
```

But the actual files written by Galexie use **`.xdr.zst`** (no `d`):

```
FC4DB5FF--62016000-62079999/FC4D8B46--62026937.xdr.zst
```

`strip_suffix(".xdr.zstd")` returns `None` → `parse_s3_key` returns `Err(InvalidS3Key)` → `mod.rs:91` `continue`s the loop → no work done → handler returns `Ok(())` in sub-1 ms.

Even if the extension were correct, the **filename format** is also wrong vs what `parse_s3_key` expects:

- **Parser expects:** `{start}-{end}.xdr.zstd` (e.g. `61827430-61827440.xdr.zstd` per the test on line 138)
- **Galexie produces:** `{hex}--{start}-{end}/{hex}--{single}.xdr.zst` (per CDK comment in `compute-stack.ts:158-159` and verified empirically)

Even after fixing the suffix, `split_once('-')` on `FC4D8B46--62026937` would produce `("FC4D8B46", "-62026937")`, and `parse::<u32>("FC4D8B46")` fails because hex isn't decimal.

### Bug 2 — tracing filter silently drops all logs

**File:** `crates/indexer/src/main.rs:13-16`

```rust
tracing_subscriber::fmt()
    .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
    .json()
    .init();
```

`EnvFilter::from_default_env()` reads `RUST_LOG`. The Lambda has **no `RUST_LOG`** env var set (verified: `aws lambda get-function-configuration` shows `RDS_PROXY_ENDPOINT`, `SECRET_ARN`, `ENV_NAME`, `BUCKET_NAME` only). When `RUST_LOG` is unset, `EnvFilter::default()` falls back to a filter that **matches nothing** — every `info!` / `warn!` / `error!` is dropped.

This is why bug 1 was invisible: the parser failure logs `warn!("skipping non-matching S3 key")` but no one sees it.

### Why CloudWatch metric `Errors = 0`

The handler's failure path is `continue` inside the loop, then `Ok(())` after the loop. Lambda runtime sees success → `Errors` stays at 0 → no alarm fires → bug went unnoticed.

## Why testing did not catch this

`crates/xdr-parser/src/lib.rs:136-156` has 4 unit tests for `parse_s3_key`:

```rust
parse_s3_key("stellar-ledger-data/ledgers/61827430-61827440.xdr.zstd")  // valid
parse_s3_key("ledgers/100-200.xdr").is_err()                             // invalid
parse_s3_key("ledgers/100200.xdr.zstd").is_err()                         // invalid
parse_s3_key("ledgers/200-100.xdr.zstd").is_err()                        // invalid
```

All tests use `.xdr.zstd` (with `d`) and a hand-invented filename pattern. **None of them use a real Galexie filename.** The parser was implemented against a fictional spec, not against the actual output of Galexie. There's no integration test that pins parser conventions to CDK filter conventions or to real Galexie outputs.

## Fix scope

1. **Rewrite `parse_s3_key`** (`crates/xdr-parser/src/lib.rs:87-129`) to accept the real Galexie format:
   - Suffix `.xdr.zst` (not `.xdr.zstd`)
   - Validate hex prefix (8 hex chars) + `--` double-dash separator (strict, prevents over-accepting)
   - Parse `{start}` (single) or `{start}-{end}` (range) after `--`
   - Update doc comment to match new format
2. **Replace tests** with real Galexie filename samples. Cover: valid single, valid range, valid with path prefix, invalid suffix, invalid hex, missing `--`, start > end, invalid number.
3. **Add `RUST_LOG: 'info'`** env var to indexer Lambda in `compute-stack.ts:146-149`.
4. **Log error on total skip** in handler (`crates/indexer/src/handler/mod.rs:77-113`): track processed count, `error!` if 0 of N records processed. This is the root cause of why the bug hid — not optional.
5. **Cross-reference comments** in parser and CDK linking suffix `.xdr.zst` to each other, so they cannot drift again.

**Out of scope (don't expand this PR):**

- Refactoring `parse_s3_key` to support arbitrary file naming schemes.
- Adding integration test infrastructure that connects S3 → Lambda → DB end-to-end (consider for follow-up task; would have caught this).
- Changing Galexie config or filename conventions.

## Acceptance criteria

- [ ] `parse_s3_key` correctly parses real Galexie filenames (suffix `.xdr.zst`, hex-prefixed, single or range).
- [ ] Tests in `crates/xdr-parser/src/lib.rs` use **actual Galexie filenames** as fixtures, not invented ones. Both single-ledger and range cases covered.
- [ ] Tests cover: valid single (`FC4D8B46--62026937.xdr.zst`), valid range, invalid suffix, invalid hex prefix, missing dashes, start > end.
- [ ] `RUST_LOG` env var set on indexer Lambda in `compute-stack.ts` so application logs are visible in CloudWatch.
- [ ] After deploy: CloudWatch logs for `staging-soroban-explorer-indexer` show `processing S3 record` lines and successful persist.
- [ ] After deploy + 5 min: `SELECT COUNT(*) FROM ledgers` returns nonzero and is increasing.
- [ ] Manual smoke check: pick one ledger sequence visible in S3, verify it appears in DB.

## Risks

- **Parser change might over-accept** — if the new regex/parser is too permissive, it could try to download non-ledger files. Mitigation: parser must require the exact `--` double-dash hex prefix that Galexie uses. Strict matching only.
- **Backfill of missed ledgers** — files have been accumulating in S3 since at least 2026-04-08T17:14 (per earlier `aws s3 ls`). Once the fix lands, indexer will only process **new** events, not retroactively process all existing files. Will need a backfill mechanism (ETL replay, manual S3 events emit, or a one-shot job iterating the bucket). **Out of scope of this bug fix** — spawn follow-up task if backfill is required.
- **Cost of `RUST_LOG=info` in prod** — INFO-level logging on every invocation increases CloudWatch ingestion cost. Acceptable for staging; revisit for production (task 0103 should configure RUST_LOG separately for prod, possibly at `warn` level).

## Related observations (not part of fix)

- CDK comment in `compute-stack.ts:130-133` is **stale**: it says "S3 events will fail and land in the DLQ. This is expected — the infra is deployed ahead of the application code." That was true before the indexer handler was implemented, but is no longer accurate (handler exists, just doesn't work for the actual file format). Comment should be removed or updated as part of this fix.
- 2076 invocations × 1 ms × 1576 bytes per S3 event payload = ~3 MB of CloudWatch ingestion + 2076 Lambda-ms. Trivial cost, but the silent failure has been burning a small amount of money for nothing.

## Coordination

- Touches lambda code → after fix lands, deploy via the standard `deploy-staging.yml` flow (with Required Reviewers gate from task 0110 still in effect).
- Does NOT touch `deploy-staging.yml` → no conflict with task 0110 in-flight PRs.

## Why high priority

This is a **production-blocking bug for staging**. The entire indexer pipeline appears to work (Galexie writing, S3 events firing, Lambda invoking, no errors) but **persists nothing**. Anyone looking at metrics or AWS console sees green; only direct DB inspection reveals the problem. Fix should land before any work that depends on staging having real data (e.g. API testing, frontend integration tests).

## Out-of-scope follow-up tasks (spawn separately if needed)

- **Backfill** historical ledgers from S3 once parser is fixed.
- **Integration test** that wires real Galexie sample files through the full S3 → Lambda → DB pipeline.
- **Lint / contract check** that pins CDK filter suffix to a constant shared with Rust parser, so they cannot drift again.
- **Alarm on persistent zero-insertion rate** — CloudWatch metric on `INSERT INTO ledgers` count from sqlx, alert if 0 over 1 hour.
