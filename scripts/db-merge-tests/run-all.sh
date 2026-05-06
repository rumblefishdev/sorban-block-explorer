#!/usr/bin/env bash
# Run T1-T5 sequentially. Each test does its own fresh_setup; final
# teardown drops every db-merge container + volume.
#
# Usage:  bash scripts/db-merge-tests/run-all.sh
# Build:  cargo build -p db-merge   (run first, run-all does NOT build)
#
# Exit code: 0 if all tests pass, nonzero if any test fails.

set -euo pipefail
LIB="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_lib.sh"
. "$LIB"

if [[ ! -x "$DB_MERGE_BIN" ]]; then
    echo "error: $DB_MERGE_BIN not found — run 'cargo build -p db-merge' first" >&2
    exit 1
fi

# Bring the full stack up once (each test will reset the DBs it touches).
bring_up postgres-truth postgres-laptop-a postgres-laptop-b postgres-merge postgres-snapshot-source

trap 'final_teardown' EXIT

failed=()
for t in T1-first-snapshot T2-single-repro T3-two-snapshot-chrono T4-idempotency T5-wrong-order; do
    if bash "$LIB_DIR/${t}.sh"; then
        :
    else
        failed+=("$t")
    fi
done

echo
if [[ ${#failed[@]} -eq 0 ]]; then
    echo "ALL TESTS PASSED (5/5)"
    exit 0
else
    echo "FAILED: ${failed[*]}"
    exit 1
fi
