#!/usr/bin/env bash
# T1: First-snapshot edge case.
# Empty postgres-merge ← snapshot of postgres-laptop-a.
# Expected: every remap table is degenerate (every natural key is new);
# diff(merge, laptop-a) returns 17/17 match.

set -euo pipefail
LIB="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_lib.sh"
. "$LIB"

echo "=== T1: First-snapshot edge case ==="

fresh_setup laptop-a merge snapshot-source

seed "$URL_LAPTOP_A" "$LIB_DIR/seed-laptop-a.sql"

dump_a="$REPO_ROOT/.temp/db-merge-fixtures/T1-laptop-a.dump"
mkdir -p "$(dirname "$dump_a")"
dump_db "$URL_LAPTOP_A" "$dump_a"

"$DB_MERGE_BIN" ingest "$dump_a" \
    --target-url "$URL_MERGE" \
    --snapshot-source-url "$URL_SNAPSHOT_SOURCE" >/dev/null

"$DB_MERGE_BIN" finalize --target-url "$URL_MERGE" >/dev/null
"$DB_MERGE_BIN" finalize --target-url "$URL_LAPTOP_A" >/dev/null

assert_diff_match "$URL_MERGE" "$URL_LAPTOP_A"

echo "=== T1 PASSED ==="
