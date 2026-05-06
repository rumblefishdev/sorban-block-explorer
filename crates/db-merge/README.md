# db-merge

Multi-laptop backfill snapshot merge tool. Implements the playbook from
[ADR 0040](../../lore/2-adrs/0040_multi-laptop-backfill-snapshot-merge-hazards.md)
and the implementation plan in task
[0186](../../lore/1-tasks/active/0186_FEATURE_db-merge-multi-laptop-snapshots.md).

## Usage

```
db-merge ingest <snapshot> --target-url <url> --snapshot-source-url <url>
db-merge finalize --target-url <url>
db-merge diff --left <url> --right <url>
```

Run `ingest` once per snapshot, **chronologically oldest-first**, against
the same target. Run `finalize` once after the last `ingest`. Use `diff`
to verify the merged target matches a sequential ground-truth backfill.

## Locked structural decisions (task 0186, Step 0)

These are decided. Do not relitigate without amending the task.

| Decision           | Choice                                                                                          |
| ------------------ | ----------------------------------------------------------------------------------------------- |
| Atomicity          | Per-table batching with `SAVEPOINT`s every 100k rows; pre-merge `pg_dump` is the rollback path  |
| Diff strategy      | Normalized natural-key projection per table → ordered → `md5_agg` → compare hashes              |
| Batching threshold | 100k rows per `INSERT … SELECT`, ledger-sequence-windowed for partitioned tables                |
| Rebuild timing     | Post-final-snapshot only — explicit `merge finalize` subcommand                                 |
| Snapshot ingestion | `pg_restore` into `postgres-snapshot-source` container; expose to target via `postgres_fdw`     |
| Language           | Rust (`crates/db-merge`); sqlx + clap; parity with `backfill-runner`                            |
| Pre-merge backup   | `pg_dump --format=custom` of target before every `merge ingest`; user removes after success     |

## Test infrastructure

Five Postgres containers in `docker-compose.yml`:

| Service                    | Port | Role                                                            |
| -------------------------- | ---- | --------------------------------------------------------------- |
| `postgres-truth`           | 5433 | Sequential ground-truth backfill of full range                  |
| `postgres-laptop-a`        | 5434 | Simulated laptop A, lower ledger range                          |
| `postgres-laptop-b`        | 5435 | Simulated laptop B, upper ledger range                          |
| `postgres-merge`           | 5436 | Merge target — receives snapshots A+B chronologically           |
| `postgres-snapshot-source` | 5437 | Ephemeral; `pg_restore` target. Reset before every `ingest`.    |

The existing `postgres` (5432) is the live backfill target — **not used in tests**.

### Reset procedures

Truncating tables is not sufficient — leaves sequence state and partition
children behind. Always drop the volume.

**Clean merge target** (between test runs):

```bash
docker compose stop postgres-merge
docker volume rm <prefix>_pgdata-merge
docker compose up -d postgres-merge
# then run migrations against postgres-merge
```

**Clean snapshot source** (between snapshots within one run): same pattern
on `postgres-snapshot-source`. `merge ingest` does this automatically as
its first step.

**Full teardown after a test session — DO NOT use `docker compose down -v`.**
The compose project mixes the live `postgres` (no profile) with the
db-merge test DBs (profile `db-merge`); `down -v` removes every project
volume including the live one regardless of which profile triggered the
command. Scope the teardown explicitly:

```bash
docker compose --profile db-merge stop \
  postgres-truth postgres-laptop-a postgres-laptop-b \
  postgres-merge postgres-snapshot-source
docker compose --profile db-merge rm -f \
  postgres-truth postgres-laptop-a postgres-laptop-b \
  postgres-merge postgres-snapshot-source
docker volume rm \
  sorban-block-explorer_pgdata-truth \
  sorban-block-explorer_pgdata-laptop-a \
  sorban-block-explorer_pgdata-laptop-b \
  sorban-block-explorer_pgdata-merge \
  sorban-block-explorer_pgdata-snapshot-source
```

## Implementation status

Phase A (skeleton) — done. Subcommands parse and log args; no merge logic
yet. See task 0186 for the full phased plan.
