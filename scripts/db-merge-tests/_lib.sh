#!/usr/bin/env bash
# Shared helpers for db-merge test scripts (T1-T5).
# Every script sources this file and uses these functions.
#
# Repo-relative paths (work from any cwd by computing relative to this lib).

set -euo pipefail

LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$LIB_DIR/../.." && pwd)"

DB_MERGE_BIN="$REPO_ROOT/target/debug/db-merge"

URL_TRUTH="postgres://postgres:postgres@localhost:5433/soroban_block_explorer"
URL_LAPTOP_A="postgres://postgres:postgres@localhost:5434/soroban_block_explorer"
URL_LAPTOP_B="postgres://postgres:postgres@localhost:5435/soroban_block_explorer"
URL_MERGE="postgres://postgres:postgres@localhost:5436/soroban_block_explorer"
URL_SNAPSHOT_SOURCE="postgres://postgres:postgres@localhost:5437/soroban_block_explorer"

PARTITIONED_TABLES=(
    transactions
    operations_appearances
    transaction_participants
    soroban_invocations_appearances
    soroban_events_appearances
    nft_ownership
    liquidity_pool_snapshots
)

bring_up() {
    cd "$REPO_ROOT"
    COMPOSE_PROFILES=db-merge docker compose up -d "$@" >/dev/null
    sleep 4
}

bring_down() {
    cd "$REPO_ROOT"
    docker compose --profile db-merge stop "$@" >/dev/null 2>&1 || true
    docker compose --profile db-merge rm -f "$@" >/dev/null 2>&1 || true
    for svc in "$@"; do
        local vol="sorban-block-explorer_pgdata-${svc#postgres-}"
        docker volume rm "$vol" >/dev/null 2>&1 || true
    done
}

# Reset a single DB to bare schema (volume drop + recreate + migrate +
# default partitions). Used between tests to start fresh.
reset_db() {
    local svc="$1"
    cd "$REPO_ROOT"
    docker compose --profile db-merge stop "$svc" >/dev/null 2>&1 || true
    docker compose --profile db-merge rm -f "$svc" >/dev/null 2>&1 || true
    docker volume rm "sorban-block-explorer_pgdata-${svc#postgres-}" >/dev/null 2>&1 || true
    COMPOSE_PROFILES=db-merge docker compose up -d "$svc" >/dev/null
    sleep 3
}

# Run all sqlx migrations against a URL. Returns nonzero on failure.
migrate() {
    local url="$1"
    cd "$REPO_ROOT"
    DATABASE_URL="$url" sqlx migrate run --source crates/db/migrations >/dev/null
}

# Create the seven `*_default` partitions on a URL (mirrors what
# db-partition-mgmt would do in prod — preflight requires them).
create_partitions() {
    local url="$1"
    local sql=""
    for t in "${PARTITIONED_TABLES[@]}"; do
        sql="${sql}CREATE TABLE ${t}_default PARTITION OF ${t} DEFAULT;"$'\n'
    done
    psql "$url" -q -v ON_ERROR_STOP=1 -c "$sql" >/dev/null
}

# Apply a SQL seed file.
seed() {
    local url="$1"
    local file="$2"
    psql "$url" -q -v ON_ERROR_STOP=1 -f "$file" >/dev/null
}

# pg_dump --format=custom helper.
dump_db() {
    local url="$1"
    local out="$2"
    pg_dump --format=custom --file="$out" "$url" >/dev/null
}

# Assert helper. $1=msg, $2=condition (evaluated as bash test).
assert() {
    local msg="$1"
    shift
    if "$@"; then
        echo "  PASS: $msg"
    else
        echo "  FAIL: $msg"
        return 1
    fi
}

# Run db-merge diff and assert all tables match.
assert_diff_match() {
    local left="$1" right="$2"
    local out
    out="$("$DB_MERGE_BIN" diff --left "$left" --right "$right" 2>&1)" || true
    if echo "$out" | grep -qE "^17/17 tables match"; then
        echo "  PASS: 17/17 tables match"
    else
        echo "  FAIL: diff did not return 17/17 match"
        echo "$out"
        return 1
    fi
}

# Setup a clean test environment: bring up needed DBs, reset all,
# migrate, partition. Pass DB short-names: "merge truth laptop-a laptop-b source".
fresh_setup() {
    local svcs=("$@")
    cd "$REPO_ROOT"
    for s in "${svcs[@]}"; do reset_db "postgres-${s}"; done
    for s in "${svcs[@]}"; do
        local var="URL_$(echo "${s//-/_}" | tr '[:lower:]' '[:upper:]')"
        local url="${!var}"
        migrate "$url"
        create_partitions "$url"
    done
}

# Final teardown — stop+remove all 5 db-merge DBs and their volumes.
# Leaves the live `postgres` (5432) untouched.
final_teardown() {
    cd "$REPO_ROOT"
    bring_down postgres-truth postgres-laptop-a postgres-laptop-b postgres-merge postgres-snapshot-source
}
