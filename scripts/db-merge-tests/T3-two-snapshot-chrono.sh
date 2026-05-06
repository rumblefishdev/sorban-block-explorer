#!/usr/bin/env bash
# T3: Two-snapshot chronological merge — the actual end-to-end correctness test.
# postgres-truth seeded with combined A+B sequential state (seed-truth.sql
# encodes what indexer would produce after backfill of full [1..6] range).
# postgres-merge ← snapshot-A then snapshot-B then finalize.
# Expected: diff(truth, merge) = 17/17.

set -euo pipefail
LIB="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_lib.sh"
. "$LIB"

echo "=== T3: Two-snapshot chronological merge ==="

fresh_setup truth laptop-a laptop-b merge snapshot-source

seed "$URL_TRUTH" "$LIB_DIR/seed-truth.sql"
seed "$URL_LAPTOP_A" "$LIB_DIR/seed-laptop-a.sql"
seed "$URL_LAPTOP_B" "$LIB_DIR/seed-laptop-b.sql"

dump_a="$REPO_ROOT/.temp/db-merge-fixtures/T3-laptop-a.dump"
dump_b="$REPO_ROOT/.temp/db-merge-fixtures/T3-laptop-b.dump"
mkdir -p "$(dirname "$dump_a")"
dump_db "$URL_LAPTOP_A" "$dump_a"
dump_db "$URL_LAPTOP_B" "$dump_b"

"$DB_MERGE_BIN" ingest "$dump_a" \
    --target-url "$URL_MERGE" \
    --snapshot-source-url "$URL_SNAPSHOT_SOURCE" >/dev/null

"$DB_MERGE_BIN" ingest "$dump_b" \
    --target-url "$URL_MERGE" \
    --snapshot-source-url "$URL_SNAPSHOT_SOURCE" >/dev/null

"$DB_MERGE_BIN" finalize --target-url "$URL_MERGE" >/dev/null

assert_diff_match "$URL_TRUTH" "$URL_MERGE"

echo "=== T3 PASSED ==="
