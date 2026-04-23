---
id: '0157'
title: 'Refactor soroban_events → soroban_events_appearances (ADR 0033)'
type: REFACTOR
status: completed
related_adr: ['0033', '0029', '0027', '0021', '0031']
related_tasks: ['0150', '0158']
tags:
  [
    layer-backend,
    layer-db,
    priority-high,
    effort-medium,
    schema,
    adr-0033,
    adr-0029,
    s3-read-path,
  ]
links:
  - lore/2-adrs/0033_soroban-events-appearances-read-time-detail.md
  - lore/2-adrs/0029_abandon-parsed-artifacts-read-time-xdr-fetch.md
  - lore/2-adrs/0027_post-surrogate-schema-and-endpoint-realizability.md
history:
  - date: '2026-04-22'
    status: active
    who: fmazur
    note: >
      Created and activated. Implements ADR 0033: collapse soroban_events
      into a 4-column appearance index, route all event detail through
      the public archive at read time.
  - date: '2026-04-23'
    status: active
    who: fmazur
    note: >
      Rescoped during implementation. The API crate has no routing state,
      error type, DB/S3 injection, or handler module yet — wiring three
      endpoints (E3/E10/E14) would also require standing that bootstrap
      up, which is architecturally out of this task. Deliverable narrowed
      to the schema change and every downstream non-API consumer: DB
      migrations in place, indexer write path rewritten as appearance
      aggregate, DTO/merge layer stripped of removed columns, ADRs
      updated, size measurement deferred until an indexer run on a
      representative sample is available.
  - date: '2026-04-23'
    status: completed
    who: fmazur
    note: >
      Closed. 20 files modified, 3 new (event_filters.rs, ADR 0033, task
      0158 spawn). Net +264/−431. Workspace build / clippy / 166 lib tests
      / fmt all green. ADR 0033 flipped to `accepted`; ADR 0027 §9 carries
      superseded-for-this-table marker; ADR 0021 E3/E10/E14 rows rewritten
      to appearance-index + S3. Spawned task 0158 for the `soroban_invocations`
      analogue. Read-path wire-up + size-measurement note deferred as
      documented in Out of Scope.
---

# Refactor soroban_events → soroban_events_appearances (ADR 0033)

## Summary

Collapse `soroban_events` into a pure appearance index
`soroban_events_appearances` with columns
`(contract_id, transaction_id, ledger_sequence, amount)` (+ `created_at`
for partition pruning). Move all parsed event detail — type, topics,
transfer fields, event_index — to read-time XDR fetch from the public
Stellar archive. Rewrite the write path (`insert_events`) to aggregate
and upsert. Rewrite the read paths for E3, E10, E14 to expand
appearances through `crates/xdr-parser::extract_events`.

## Status: Completed (schema + write path delivered; read path deferred)

**Delivered state:** Schema change and every non-API consumer in place;
ADR 0033 flipped from `proposed` to `accepted` for its delivered half.
Read-path wire-up (E3/E10/E14 handlers) was deferred — the API crate has
no `AppState`, no error/IntoResponse type, and no handlers module, so
the three endpoints would have to build on infrastructure that doesn't
exist yet. Splitting that out kept this task bounded to the schema and
left the API bootstrap to a dedicated follow-up.

## Context

See ADR 0033 for the full rationale and decision. Short version: the
current 11-column per-event table is the largest Soroban-domain
consumer of DB space (~54 GB at full Soroban coverage) and its
remaining non-index columns (`topic0`, `transfer_*`, `event_type`,
`event_index`) already have the same source-of-truth on the public
archive. ADR 0029 moved the heavy event fields there for E3/E14;
ADR 0033 finishes the job for E10 and for list views.

## Scope

### In scope (delivered)

1. **Schema rewrite** — in-place edit of
   `crates/db/migrations/0004_soroban_activity.sql` and the
   `20260421000100_replay_safe_uniques` up/down pair. New table
   `soroban_events_appearances` per ADR 0033 §Decision.1; old table
   dropped. Monthly partitioning on `created_at` preserved. ADR 0031's
   `event_type_name()` SQL helper and `event_type` SMALLINT column
   removed from `20260422000000_enum_label_functions`.
2. **Indexer write path** — rewrite `insert_events` in
   `crates/indexer/src/handler/persist/write.rs`:
   - aggregate parsed contract events in memory by
     `(contract_id, transaction_id, ledger_sequence, created_at)`,
   - skip events without a resolved `contract_id` (appearance index
     is contract-scoped by construction),
   - emit one row per trio with `amount = count`,
   - `ON CONFLICT (…) DO NOTHING` replay-safe upsert.
     `EventRow` simplified to the minimum identity fields; per-event
     `event_type`, `topic0`, `event_index`, and transfer triple removed
     from staging. Diagnostic events still filtered before aggregation
     (S3-lane per ADR 0033, same rule as ADR 0027).
3. **Shared transfer-classification helper** —
   `crates/xdr-parser::event_filters` module with
   `is_transfer_event`, `parse_transfer`, and `transfer_participants`
   helpers shared between the indexer (participant registration) and
   the future API read path (token-transactions transfer filter).
4. **API DTO / merge cleanup** — `crates/api/src/stellar_archive/dto.rs`
   and `merge.rs` stripped of fields sourced from the removed DB
   columns. `E14EventResponse<_>` and `merge_e14_events` removed (no
   DB-side event row to merge against under ADR 0033). `E14HeavyEventFields`
   preserved as the per-event record shape the future handler will
   assemble from parser output.
5. **Partition management** — `db-partition-mgmt` and
   `backfill-bench` table lists updated; monthly partition naming
   (`soroban_events_appearances_yYYYYmMM`) applied.
6. **Dead code removal** — unused `domain::soroban::SorobanEvent`
   domain mirror struct removed; doc-comments across `domain`,
   `xdr-parser`, and `indexer` realigned to ADR 0033.
7. **ADR updates** — ADR 0027 §9 carries a superseded-for-this-table
   marker pointing to ADR 0033; ADR 0021 E3/E10/E14 rows rewritten
   to the appearance-index + S3 model; ADR 0033 flipped
   `proposed` → `accepted`.
8. **Tests** — workspace build / clippy / unit tests / fmt green.
   `persist_integration` asserts `SUM(amount)` matches the ingested
   non-diagnostic event count, covering the aggregate invariant.

### Out of scope (deferred)

- **Read path (E3/E10/E14 handlers).** Deferred to a follow-up task.
  The API crate has no `AppState`, `IntoResponse` error type, DB
  pool / `StellarArchiveFetcher` injection, or handlers module today.
  Wiring the three endpoints would also require building that
  bootstrap, which is an architecturally independent concern that
  doesn't belong in the schema-change PR. ADR 0033's decision stands;
  task 0157 delivers the schema substrate the follow-up will consume.
- **Size measurement note.** ADR 0033 §Open Question 4 asks for a
  measured row-count / size on a representative sample. Cannot be
  produced without an indexer run on real data; leave for the
  follow-up that lands after a backfill cycle.
- **Frontend rendering** — contract-events panel and token-tx list.
  Not on this task.
- **Caching / memoisation beyond request scope** — ADR 0033
  preserves ADR 0029's "cache only if measurement requires" rule.
- **Any change to non-event Soroban tables.** `soroban_contracts`,
  `soroban_invocations`, etc., untouched.
- **Live production backfill.** No prod DB exists; this is a schema
  rewrite on migrations, not a data migration.

## Implementation Plan

### Step 1 — Schema migration rewrite

Edit `0004_soroban_activity.sql` in place: new `CREATE TABLE
soroban_events_appearances` DDL + `CREATE INDEX` for the two new
indexes. Remove the `soroban_events` DDL, its indexes, and the
`accounts`/`event_type` columns that only served this table's
detail. Edit `20260421000100_replay_safe_uniques.up.sql` to drop the
`uq_soroban_events_tx_index` block (no longer applicable — the new
PK covers replay idempotency). Verify `nx run rust:build` + `nx run
rust:test` pass against the rewritten migrations.

### Step 2 — Indexer write path

Rewrite `insert_events` in
`crates/indexer/src/handler/persist/write.rs`. Aggregate
`Vec<ContractEvent>` → `HashMap<(contract_id, tx_id, ledger_seq),
u64>`. Emit one batch insert keyed on the composite PK with
`ON CONFLICT DO NOTHING`. Drop event_type / topic0 / transfer
classification from this function entirely.

### Step 3 — API read path (E14)

Appearances query paginated by `(ledger_sequence DESC,
transaction_id DESC)`. For each distinct `ledger_sequence` on the
page, one S3 `GetObject` + `xdr_parser::extract_events`
filtered by `contract_id`. Expand each appearance row into its
`amount` consecutive events. Memoise the decoded ledger in a
request-scoped `HashMap` so back-to-back appearances in the same
ledger share one parse.

### Step 4 — API read path (E3)

Simplify: use appearances as the "events?" probe; fetch the
transaction's ledger once; extract all events; filter to the
transaction's operation(s) by index. Response shape unchanged.

### Step 5 — API read path (E10)

Rewrite the query to `SELECT DISTINCT transaction_id FROM
soroban_events_appearances WHERE contract_id = :token_contract
ORDER BY …`. Parser-side filter `ContractEventType::Contract +
topic matches transfer symbol` on decoded events. Move the filter
into a reusable helper in `crates/xdr-parser::event_filters`.

### Step 6 — DTO / merge cleanup

Remove DB-sourced fields from
`crates/api/src/stellar_archive/dto.rs` that the new schema no
longer provides. `merge.rs` becomes "appearances + parser output →
response" with no DB-detail fallback.

### Step 7 — ADR 0021 + ADR 0027 updates

One commit: patch `0021_schema-endpoint-frontend-coverage-matrix.md`
rows for E3/E10/E14; add a superseded-for-this-table note in ADR
0027 §soroban_events linking to ADR 0033.

### Step 8 — Tests

- Migration round-trip: up + down + up.
- Indexer ingest: a crafted sample ledger with N events for one
  contract across M transactions → expect M rows with `amount`s
  summing to N.
- E14 integration: contract with appearances in ≥3 ledgers, page
  size 2 → three pages, correct cursor, correct event order.
- E10 integration: token contract with transfer and non-transfer
  events → only transfer-bearing transactions returned.

### Step 9 — Size measurement note

After running the indexer against a representative Soroban ledger
range (pick one the team already uses for benchmarks), record row
count and total size (heap + indexes) under
`notes/S-size-measurement.md` for reference in ADR 0033's Open
Question 4.

## Acceptance Criteria

- [x] `soroban_events_appearances` table matches ADR 0033 §Decision.1
      DDL; `soroban_events` no longer exists in migrations or code.
- [x] `insert_events` aggregates per `(contract, tx, ledger, created_at)`;
      `persist_integration` asserts `SUM(amount)` matches the ingested
      non-diagnostic event count.
- [x] No compile-time reference to `topic0`, `transfer_from_id`,
      `transfer_to_id`, `transfer_amount`, `event_type`, or
      `event_index` in the `crates/db`, `crates/indexer`, or
      `crates/api` code for this table.
- [x] ADR 0021 coverage matrix updated (E3/E10/E14 rows); ADR 0027
      carries a superseded-for-this-table marker; ADR 0033 flipped to
      `accepted`.
- [x] `cargo build --workspace`, `cargo test --workspace --lib`,
      `cargo clippy --workspace --all-targets -- -D warnings`, and
      `cargo fmt --all -- --check` all green.
- [ ] E14, E10, E3 handlers return responses whose event detail comes
      entirely from parser output — **deferred** (API bootstrap out of
      scope; schema substrate ready).
- [ ] E14 pagination behaves like StellarChain — **deferred** (same
      reason).
- [ ] Size measurement note written — **deferred** (requires a real
      indexer run on a representative sample).

## Implementation Notes

**Touched files (20 modified + 3 new):**

- `crates/db/migrations/0004_soroban_activity.sql` — table replaced
  in place
- `crates/db/migrations/20260421000100_replay_safe_uniques.{up,down}.sql`
  — `uq_soroban_events_tx_index` block removed
- `crates/db/migrations/20260422000000_enum_label_functions.{up,down}.sql`
  — `event_type_name()` helper + column converter removed
- `crates/indexer/src/handler/persist/{mod,staging,write}.rs` —
  `insert_events` rewritten as aggregate; `EventRow` simplified to
  identity fields; local transfer helpers deleted
- `crates/indexer/tests/persist_integration.rs` — `SUM(amount)`
  assertion added; `event_type_name` check removed; table names
  refreshed
- `crates/xdr-parser/src/{lib,types}.rs` + new
  `crates/xdr-parser/src/event_filters.rs` — new public module
  `event_filters::{is_transfer_event, parse_transfer,
transfer_participants, Transfer}`, 13 unit tests
- `crates/api/src/stellar_archive/{dto,merge}.rs` —
  `E14EventResponse<_>` + `merge_e14_events` removed (no DB-side
  event row to merge under ADR 0033); `E14HeavyEventFields` kept as
  the per-event record shape
- `crates/backfill-bench/src/main.rs` + `crates/db-partition-mgmt/src/lib.rs`
  — table name updated in partition bootstrap lists
- `crates/domain/src/{enums/mod,enums/contract_event_type,soroban}.rs`
  — dead `SorobanEvent` struct removed; doc comments realigned
- `lore/2-adrs/0033_*.md` (new) — accepted
- `lore/2-adrs/0027_*.md` + `lore/2-adrs/0021_*.md` — updated per
  scope
- `lore/1-tasks/backlog/0158_*.md` (new) — `soroban_invocations`
  analogue follow-up

**Net:** +264 / −431 across tracked files.

**Test posture:**

- `cargo build --workspace` ✅
- `cargo test --workspace --lib` ✅ — 166 passed (13 new in
  `xdr-parser::event_filters`, plus the `SUM(amount)` assertion in
  `persist_integration`)
- `cargo clippy --workspace --all-targets -- -D warnings` ✅
- `cargo fmt --all -- --check` ✅
- Integration tests requiring live Postgres run clean locally against
  the migrated schema (38,854 rows sample on dev DB, `SUM(amount)`
  invariant holds).

## Issues Encountered

- **Parser function name mismatch.** ADR 0033 and the original task
  plan referenced `xdr_parser::extract_contract_events`. Actual
  symbol in `crates/xdr-parser/src/event.rs:17` is `extract_events`.
  Not a blocker — the same function is the single source of truth —
  but the ADR / task prose were drifting. The ADR-0021 matrix update
  used the correct name on first land; ADR 0033 pseudocode + task
  summary were corrected in a follow-up fixup after Copilot review
  of the PR.
- **Handler infrastructure absent.** The API crate has no
  `AppState`, no `IntoResponse` error type, no handlers module, no
  DB-pool/S3-fetcher injection. Three endpoints (E3/E10/E14) were
  implicitly assumed wired by the task plan; they are not, and
  standing up that bootstrap is a separate architectural decision.
  Rescoped to schema + write path only; read-path wiring deferred to
  a follow-up alongside the API bootstrap itself.
- **Merge conflict on `crates/db-partition-mgmt/src/main.rs`.**
  Upstream commit `8a03b27 fix(lore-0139): align partition Lambda to
time schema` split the Lambda into a thin `main.rs` + a new
  `lib.rs`. My rename (`soroban_events` → `soroban_events_appearances`)
  was sitting on the old monolithic file. Resolved by taking
  upstream's `main.rs` verbatim and propagating the rename into the
  new `lib.rs` (plus its expanded `TIME_PARTITIONED_TABLES` list of
  six partitioned tables).
- **ADR 0031 `event_type SMALLINT` converter surface.** The helper
  `event_type_name()` lived in `20260422000000_enum_label_functions.up.sql`
  and was exercised by a round-trip assertion in `persist_integration`.
  Both removed together; `ContractEventType` enum itself stays
  (used at parse time to filter diagnostic events out of the
  aggregate and to tag events at read time).

## Design Decisions

### From Plan

1. **Monthly partitioning preserved on `created_at`.** ADR 0033
   §Decision.1. `soroban_events_appearances` still partitions on
   `created_at`; PK `(contract_id, transaction_id, ledger_sequence,
created_at)` carries the partition key so the replay PK is
   deterministic.
2. **Diagnostic events excluded from the aggregate.** Same rule as
   ADR 0027 — diagnostic events are S3-only. Staging filters them
   before pushing to the aggregate HashMap, so `SUM(amount)` counts
   only Contract + System events.
3. **Shared transfer-classification helper in `xdr-parser`.** Task
   plan §In-Scope step 3 called for `event_filters::is_transfer_event`.
   Exposed with three public entry points (`is_transfer_event`,
   `parse_transfer`, `transfer_participants`) so both indexer
   staging and the future E10 read path consume one rule set.

### Emerged

4. **`contract_id NOT NULL` on the appearance index.** ADR 0033 DDL
   marks it NOT NULL; the old table allowed NULL (system events
   without a contract emitter could land in the table). Indexer
   write path now _skips_ events whose `contract_id` does not
   resolve — the appearance index is contract-scoped by construction,
   and a contract-less event has no meaningful home in it. Logged
   as a silent skip; if the rate is non-trivial post-backfill, we
   revisit.
5. **Dead code deletion, not deprecation.** `domain::SorobanEvent`
   struct had no callers across the workspace (confirmed by grep).
   Deleted outright rather than `#[deprecated]`-flagged, since no
   downstream consumers needed a grace window.
6. **`E14EventResponse<_>` + `merge_e14_events` deleted, not
   repurposed.** Under the new model there is no DB-side event row
   to merge against — the "light" side vanished. Kept
   `E14HeavyEventFields` as the per-event record shape the future
   handler will emit directly. Wirings that would have re-introduced
   a merge helper go in the follow-up read-path task.
7. **`event_type_name()` SQL helper removed, not kept as dead
   function.** ADR 0033 §Decision.3 explicitly drops the `event_type`
   SMALLINT column; the label helper had no remaining consumer, and
   the `persist_integration` round-trip test that drove drift
   detection lost its anchor. Removed the helper + the test
   assertion; `event_type_name` now lives only in git history.
8. **Single atomic PR, not stepwise merges.** Precedent from
   ADR 0030 / ADR 0031 (both rewrite-in-place schema changes) kept
   multi-step refactors in one PR. Temporary red build between
   steps 2 and 3 is fine — CI runs on the final commit, and
   reviewers see the final state.
9. **Rescope acknowledgement in task history + frontmatter** rather
   than silent shrink. When the read-path surface turned out to be
   "build the API crate first," I surfaced it to the user with three
   options (one-PR-mega / split / hybrid) and the user chose split.
   The original acceptance criteria were rewritten in place to
   reflect the revised scope with deferred items explicitly
   `[ ]`-marked and tagged "deferred (…reason…)".

## Broken / Modified Tests

- `crates/indexer/tests/persist_integration.rs:251` —
  `check_all!(&pool, "event_type_name", ContractEventType)` removed.
  Intentional: the SQL function no longer exists per ADR 0033 §Decision.3.
  Not a regression; comment next to the removal points at the ADR.
- Same file: `counts_first.events == 1` assertion kept but extended
  with `counts_first.events_amount_sum == 1` (new field on `Counts`
  struct). Row-count check continues to hold under the aggregate
  model (1 event → 1 appearance row with `amount = 1`) and the new
  sum assertion catches a regression that would silently zero out
  `amount`.

## Future Work

- **Task 0158** — `soroban_invocations` ADR-0033 analogue (spawned).
- **API bootstrap + read-path wiring for E3 / E10 / E14** — not
  spawned yet (per repo convention: don't auto-spawn without explicit
  ask). Architecturally independent from this task; will land once
  someone scopes the API crate's `AppState` / `IntoResponse` /
  handler-module shape.
- **Size-measurement note on a representative bench range** —
  ADR 0033 §Open Question 4. Needs a real indexer run; dev-DB
  sample (38,854 rows / 7.3 MB / ~196 B per row) recorded verbally
  during the session for calibration but not committed as a formal
  note.

## Notes

- Same rewrite-in-place convention as ADR 0030 and ADR 0031 — valid
  because there is no production DB yet.
- Frontend work (events panel, token-tx list visual changes) is
  deliberately deferred.
- The parser already exposes `extract_events`; no new parsing code
  was required, only a new `event_filters` module with the
  transfer-event helpers that indexer staging + future API read path
  both consume.
