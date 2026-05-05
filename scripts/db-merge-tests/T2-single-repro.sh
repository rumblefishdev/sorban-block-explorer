#!/usr/bin/env bash
# T2: Single-snapshot reproducibility.
# postgres-truth seeded with the SAME data laptop-a has (modeling
# sequential backfill of laptop-a's range alone). postgres-merge ←
# snapshot of laptop-a. Expected: diff(truth, merge) = 17/17.
#
# With synthetic seeds T2 is structurally close to T1 (truth = laptop-a
# data-wise); the test still verifies the merge produces what sequential
# backfill produces.

set -euo pipefail
LIB="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_lib.sh"
. "$LIB"

echo "=== T2: Single-snapshot reproducibility ==="

fresh_setup truth laptop-a merge snapshot-source

seed "$URL_TRUTH" "$LIB_DIR/seed-laptop-a.sql"
seed "$URL_LAPTOP_A" "$LIB_DIR/seed-laptop-a.sql"

dump_a="$REPO_ROOT/.temp/db-merge-fixtures/T2-laptop-a.dump"
mkdir -p "$(dirname "$dump_a")"
dump_db "$URL_LAPTOP_A" "$dump_a"

"$DB_MERGE_BIN" ingest "$dump_a" \
    --target-url "$URL_MERGE" \
    --snapshot-source-url "$URL_SNAPSHOT_SOURCE" >/dev/null

"$DB_MERGE_BIN" finalize --target-url "$URL_MERGE" >/dev/null
"$DB_MERGE_BIN" finalize --target-url "$URL_TRUTH" >/dev/null

assert_diff_match "$URL_TRUTH" "$URL_MERGE"

echo "=== T2 PASSED ==="
