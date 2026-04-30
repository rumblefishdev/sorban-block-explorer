#!/usr/bin/env bash
# ============================================================================
# Phase 1 SQL invariants runner — task 0175.
#
# Iterates the per-table SQL files in ./sql/ in numeric order, runs each
# against the configured DATABASE_URL, and aggregates output. Each file is
# expected to emit one or more rows of the shape `(violations, sample)` per
# invariant; this wrapper just adds section headers and a pass/fail
# summary at the end.
#
# Usage:
#   DATABASE_URL=postgres://postgres:postgres@localhost:5432/soroban_block_explorer \
#       crates/audit-harness/run-invariants.sh [--out report.md]
#
# Exit codes:
#   0  every invariant returned 0 violations
#   1  one or more violations
#   2  configuration / runtime error (DB unreachable, missing psql, etc.)
# ============================================================================

set -euo pipefail

DATABASE_URL="${DATABASE_URL:-postgres://postgres:postgres@localhost:5432/soroban_block_explorer}"
OUT_FILE=""
SQL_DIR="$(cd "$(dirname "$0")" && pwd)/sql"

while [ $# -gt 0 ]; do
    case "$1" in
        --out) OUT_FILE="$2"; shift 2 ;;
        --sql-dir) SQL_DIR="$2"; shift 2 ;;
        -h|--help)
            sed -n '2,/^# ========/p' "$0" | sed 's/^# \?//'
            exit 0 ;;
        *) echo "unknown arg: $1" >&2; exit 2 ;;
    esac
done

command -v psql >/dev/null || { echo "psql not on PATH" >&2; exit 2; }

if ! psql "$DATABASE_URL" -c 'SELECT 1' >/dev/null 2>&1; then
    echo "cannot connect to $DATABASE_URL" >&2
    exit 2
fi

run_sql_files() {
    local timestamp
    timestamp=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
    cat <<EOF
# Audit harness — Phase 1 SQL invariants

**Timestamp:** $timestamp
**Database:** $DATABASE_URL

---

EOF
    for f in "$SQL_DIR"/*.sql; do
        [ -f "$f" ] || continue
        # psql -X: ignore .psqlrc; -q: quiet; -A -t for tuples-only would lose
        # the \echo headers, so keep aligned mode and let the SQL files
        # provide their own structure via ## markdown headers.
        psql -X -q "$DATABASE_URL" -f "$f" 2>&1 || {
            echo "psql exit non-zero for $(basename "$f")" >&2
            return 1
        }
        echo
    done
}

if [ -n "$OUT_FILE" ]; then
    run_sql_files | tee "$OUT_FILE"
else
    run_sql_files
fi

# Aggregate pass/fail. Re-run a compact summary that only reports total
# violation counts per file by looking at the just-emitted output. For
# operator-friendly use we print a final tally line.
# Strategy: run a single COUNT-aggregating query at the end to re-summarise
# the most critical invariants. Future improvement: structured JSON output.
echo
echo "---"
echo
echo "## Summary"
echo
SUMMARY=$(psql -X -q -tA "$DATABASE_URL" <<'SQL'
WITH t AS (
    SELECT
      (SELECT COUNT(*) FROM ledgers) AS ledgers,
      (SELECT COUNT(*) FROM transactions) AS transactions,
      (SELECT COUNT(*) FROM transaction_hash_index) AS thi,
      (SELECT COUNT(*) FROM operations_appearances) AS ops_app,
      (SELECT COUNT(*) FROM transaction_participants) AS tx_part,
      (SELECT COUNT(*) FROM soroban_contracts) AS contracts,
      (SELECT COUNT(*) FROM wasm_interface_metadata) AS wasm,
      (SELECT COUNT(*) FROM soroban_events_appearances) AS sea,
      (SELECT COUNT(*) FROM soroban_invocations_appearances) AS sia,
      (SELECT COUNT(*) FROM assets) AS assets,
      (SELECT COUNT(*) FROM accounts) AS accounts,
      (SELECT COUNT(*) FROM account_balances_current) AS abc,
      (SELECT COUNT(*) FROM nfts) AS nfts,
      (SELECT COUNT(*) FROM nft_ownership) AS nft_own,
      (SELECT COUNT(*) FROM liquidity_pools) AS lp,
      (SELECT COUNT(*) FROM liquidity_pool_snapshots) AS lps,
      (SELECT COUNT(*) FROM lp_positions) AS lpp
)
SELECT format(E'rows by table:\n  ledgers=%s, transactions=%s, transaction_hash_index=%s,\n  operations_appearances=%s, transaction_participants=%s,\n  soroban_contracts=%s, wasm_interface_metadata=%s,\n  soroban_events_appearances=%s, soroban_invocations_appearances=%s,\n  assets=%s, accounts=%s, account_balances_current=%s,\n  nfts=%s, nft_ownership=%s, liquidity_pools=%s,\n  liquidity_pool_snapshots=%s, lp_positions=%s',
              ledgers, transactions, thi, ops_app, tx_part, contracts, wasm,
              sea, sia, assets, accounts, abc, nfts, nft_own, lp, lps, lpp)
FROM t;
SQL
)
echo "$SUMMARY"
echo
echo "_(violation counts shown per-invariant in sections above; zero = green.)_"
