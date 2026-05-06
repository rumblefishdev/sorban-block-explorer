#!/usr/bin/env bash
# T4: Idempotency.
# After T3, re-run merge ingest <snapshot-a> and merge ingest <snapshot-b>
# (replay) with --allow-overlap (chronological gate would otherwise abort).
# Expected: zero new rows in any table; diff still 17/17.

set -euo pipefail
LIB="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_lib.sh"
. "$LIB"

echo "=== T4: Idempotency (replay) ==="

fresh_setup truth laptop-a laptop-b merge snapshot-source

seed "$URL_TRUTH" "$LIB_DIR/seed-truth.sql"
seed "$URL_LAPTOP_A" "$LIB_DIR/seed-laptop-a.sql"
seed "$URL_LAPTOP_B" "$LIB_DIR/seed-laptop-b.sql"

dump_a="$REPO_ROOT/.temp/db-merge-fixtures/T4-laptop-a.dump"
dump_b="$REPO_ROOT/.temp/db-merge-fixtures/T4-laptop-b.dump"
mkdir -p "$(dirname "$dump_a")"
dump_db "$URL_LAPTOP_A" "$dump_a"
dump_db "$URL_LAPTOP_B" "$dump_b"

# Initial merge (chronological)
"$DB_MERGE_BIN" ingest "$dump_a" --target-url "$URL_MERGE" --snapshot-source-url "$URL_SNAPSHOT_SOURCE" >/dev/null
"$DB_MERGE_BIN" ingest "$dump_b" --target-url "$URL_MERGE" --snapshot-source-url "$URL_SNAPSHOT_SOURCE" >/dev/null
"$DB_MERGE_BIN" finalize --target-url "$URL_MERGE" >/dev/null

# Capture per-table row counts as snapshot of "post-T3 state".
counts_before="$(psql "$URL_MERGE" -tA -c "
    SELECT relname || ':' || n_live_tup
      FROM pg_stat_user_tables
     WHERE schemaname = 'public'
     ORDER BY relname")"

# Replay both with --allow-overlap.
"$DB_MERGE_BIN" ingest "$dump_a" --target-url "$URL_MERGE" --snapshot-source-url "$URL_SNAPSHOT_SOURCE" --allow-overlap >/dev/null
"$DB_MERGE_BIN" ingest "$dump_b" --target-url "$URL_MERGE" --snapshot-source-url "$URL_SNAPSHOT_SOURCE" --allow-overlap >/dev/null
"$DB_MERGE_BIN" finalize --target-url "$URL_MERGE" >/dev/null

# pg_stat_user_tables is autovacuum-driven; force ANALYZE so n_live_tup
# is fresh before the comparison.
psql "$URL_MERGE" -q -c "ANALYZE" >/dev/null
counts_after="$(psql "$URL_MERGE" -tA -c "
    SELECT relname || ':' || n_live_tup
      FROM pg_stat_user_tables
     WHERE schemaname = 'public'
     ORDER BY relname")"

# Also verify ground-truth still matches (the watermark/dedup paths hold).
"$DB_MERGE_BIN" finalize --target-url "$URL_TRUTH" >/dev/null

if [[ "$counts_before" == "$counts_after" ]]; then
    echo "  PASS: row counts unchanged after replay"
else
    echo "  FAIL: row counts changed after replay"
    diff <(echo "$counts_before") <(echo "$counts_after") || true
    exit 1
fi
assert_diff_match "$URL_TRUTH" "$URL_MERGE"

echo "=== T4 PASSED ==="
