#!/usr/bin/env bash
# T5: Wrong order rejected.
# After ingesting laptop-b (ledgers 4..6), attempt to ingest laptop-a
# (ledgers 1..3). Expected: pre-flight aborts with chronological-only
# violation message; merge target unchanged.

set -euo pipefail
LIB="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_lib.sh"
. "$LIB"

echo "=== T5: Wrong order rejected ==="

fresh_setup laptop-a laptop-b merge snapshot-source

seed "$URL_LAPTOP_A" "$LIB_DIR/seed-laptop-a.sql"
seed "$URL_LAPTOP_B" "$LIB_DIR/seed-laptop-b.sql"

dump_a="$REPO_ROOT/.temp/db-merge-fixtures/T5-laptop-a.dump"
dump_b="$REPO_ROOT/.temp/db-merge-fixtures/T5-laptop-b.dump"
mkdir -p "$(dirname "$dump_a")"
dump_db "$URL_LAPTOP_A" "$dump_a"
dump_db "$URL_LAPTOP_B" "$dump_b"

# Ingest B first (target empty → preflight passes).
"$DB_MERGE_BIN" ingest "$dump_b" --target-url "$URL_MERGE" --snapshot-source-url "$URL_SNAPSHOT_SOURCE" >/dev/null

# Now attempt A — should abort with "source range precedes or overlaps".
set +e
out="$("$DB_MERGE_BIN" ingest "$dump_a" --target-url "$URL_MERGE" --snapshot-source-url "$URL_SNAPSHOT_SOURCE" 2>&1)"
rc=$?
set -e

if [[ $rc -eq 0 ]]; then
    echo "  FAIL: ingest unexpectedly succeeded"
    echo "$out"
    exit 1
fi
if echo "$out" | grep -q "chronological-only contract violated"; then
    echo "  PASS: preflight rejected wrong-order ingest"
else
    echo "  FAIL: ingest failed but not for the expected reason"
    echo "$out"
    exit 1
fi

echo "=== T5 PASSED ==="
