#!/usr/bin/env bash
#
# run_endpoint.sh — run any of the 23 endpoint-queries SQL files against the
# local Docker Postgres.
#
# Assumes:
#   • the DB container is up (`docker ps` shows it)
#   • the schema migrations are applied
#   • the tables are populated (otherwise some discovery lookups return empty
#     and the corresponding endpoint is reported and skipped)
#
# Each `case` branch corresponds to exactly one .sql file from this directory.
# Discovery for required inputs (account StrKey, contract StrKey, tx hash,
# pool id, etc.) is done up-front against the live DB so the script picks
# real values without you having to look them up by hand.

set -uo pipefail
# `-e` is intentionally NOT set — `all` mode is allowed to soldier on past
# individual endpoint failures so you see the full picture in one run.

# =============================================================================
# Config (override via env)
# =============================================================================
DB_USER="${SBE_PG_USER:-postgres}"
DB_NAME="${SBE_PG_DB:-soroban_block_explorer}"
QUERY_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Use `docker compose exec` against the repo's compose file rather than a
# hard-coded container name. The container name varies with the directory
# the repo is cloned into ("sorban-…" vs "soroban-…" depending on history),
# so addressing the service by its compose name (`postgres`) is portable.
# `SBE_COMPOSE_FILE` lets a CI / unusual layout override the path.
COMPOSE_FILE="${SBE_COMPOSE_FILE:-$(git -C "$QUERY_DIR" rev-parse --show-toplevel 2>/dev/null)/docker-compose.yml}"
COMPOSE_SERVICE="${SBE_PG_SERVICE:-postgres}"

EXPLAIN_PREFIX=""
EXPANDED=0

# =============================================================================
# Help
# =============================================================================
usage() {
    local me; me=$(basename "$0")
    cat <<EOF
Usage: $me <id> [--explain]

Run one of the 23 endpoint-queries SQL files against the local Docker Postgres.

IDs:
  01..23     run a single endpoint
  all        run every endpoint in sequence

Flags:
  --explain      wrap the reference query in EXPLAIN (ANALYZE, BUFFERS)
  -x, --expanded use psql expanded output (\x on) — one key=value per line.
                 Recommended for wide rows (E2/E3/E7/E10/E13/E14/E20).
  -h, --help

Env:
  SBE_COMPOSE_FILE  default: <repo-root>/docker-compose.yml
  SBE_PG_SERVICE    default: postgres
  SBE_PG_USER       default: postgres
  SBE_PG_DB         default: soroban_block_explorer

Examples:
  ./$me 01                  # network stats
  ./$me 04                  # ledgers list (50 newest)
  ./$me 02 -x               # transactions list, expanded (key=value per line)
  ./$me 03 --explain        # /transactions/:hash with EXPLAIN
  ./$me 02 | less -S        # aligned table, horizontal scroll in less
  ./$me all                 # smoke-run every endpoint
EOF
}

# =============================================================================
# Argument parsing
# =============================================================================
ID=""
for arg in "$@"; do
    case "$arg" in
        --explain) EXPLAIN_PREFIX="EXPLAIN (ANALYZE, BUFFERS)" ;;
        -x|--expanded) EXPANDED=1 ;;
        -h|--help) usage; exit 0 ;;
        *)
            if [[ -z "$ID" ]]; then ID="$arg"
            else echo "unexpected arg: $arg" >&2; usage; exit 1
            fi
            ;;
    esac
done
[[ -z "$ID" ]] && { usage; exit 1; }

# =============================================================================
# psql wrappers
# =============================================================================
psql_pipe() {
    # Multi-line script via stdin; output to caller's stdout.
    local extra=()
    [[ "$EXPANDED" == "1" ]] && extra+=("-x")
    docker compose -f "$COMPOSE_FILE" exec -T "$COMPOSE_SERVICE" psql \
        -U "$DB_USER" -d "$DB_NAME" \
        -v ON_ERROR_STOP=1 "${extra[@]}"
}

psql_oneshot() {
    # One-shot SELECT, value-only output for capture in shell.
    docker compose -f "$COMPOSE_FILE" exec -T "$COMPOSE_SERVICE" psql \
        -U "$DB_USER" -d "$DB_NAME" \
        -v ON_ERROR_STOP=1 -t -A -c "$1"
}

# =============================================================================
# SQL file helpers
# =============================================================================

# get_statement <file> <n> — print the n-th statement (1-indexed) from a
# multi-statement file split on `-- @@ split @@`. For single-statement files
# n=1 returns the whole file.
get_statement() {
    local file="$1" n="$2"
    awk -v n="$n" '
        BEGIN { stmt = 1 }
        /^-- @@ split @@/ { stmt++; next }
        { if (stmt == n) print }
    ' "$file"
}

# substitute_params <sql> <p1> <p2> ... — replace $1, $2, … in the SQL with
# the listed values (literal text). Iterates from highest index down so that
# $10 is replaced before $1 (avoids partial matches). Values must already be
# valid SQL literals or `NULL::type` casts.
substitute_params() {
    local sql="$1"; shift
    local args=("$@")
    local i
    for ((i=${#args[@]}; i>=1; i--)); do
        local val="${args[$((i-1))]}"
        sql="${sql//\$$i/$val}"
    done
    printf '%s' "$sql"
}

# wrap a SQL statement in EXPLAIN (ANALYZE, BUFFERS) iff --explain was passed
explain_wrap() {
    local sql="$1"
    if [[ -n "$EXPLAIN_PREFIX" ]]; then
        printf '%s %s' "$EXPLAIN_PREFIX" "$sql"
    else
        printf '%s' "$sql"
    fi
}

# =============================================================================
# Discovery
# =============================================================================
require_value() {
    local v="$1" label="$2"
    if [[ -z "$v" ]]; then
        echo "  SKIP: $label is empty in DB; populate the table first." >&2
        return 1
    fi
    return 0
}

# =============================================================================
# Per-endpoint runners
# =============================================================================
run_one() {
    local id="$1"
    local FILE STMT SUB

    case "$id" in

    # -------------------------------------------------------------------------
    01)
        echo "=== E01: GET /network/stats ==="
        FILE="$QUERY_DIR/01_get_network_stats.sql"
        explain_wrap "$(<"$FILE")" | psql_pipe
        ;;

    # -------------------------------------------------------------------------
    02)
        echo "=== E02: GET /transactions (statement A — no filter) ==="
        FILE="$QUERY_DIR/02_get_transactions_list.sql"
        STMT=$(get_statement "$FILE" 1)
        SUB=$(substitute_params "$STMT" \
            "50" "NULL::timestamptz" "NULL::bigint" \
            "NULL::bigint" "NULL::bigint" "NULL::smallint")
        explain_wrap "$SUB" | psql_pipe
        ;;

    # -------------------------------------------------------------------------
    03)
        echo "=== E03: GET /transactions/:hash ==="
        local hex
        hex=$(psql_oneshot "
            SELECT encode(hash, 'hex')
            FROM transaction_hash_index
            ORDER BY ledger_sequence DESC LIMIT 1;
        ")
        require_value "$hex" "transaction_hash_index" || return 1
        echo "  hash = $hex"

        FILE="$QUERY_DIR/03_get_transactions_by_hash.sql"
        local sa sb sc sd se sf
        sa=$(get_statement "$FILE" 1)
        sb=$(get_statement "$FILE" 2)
        sc=$(get_statement "$FILE" 3)
        sd=$(get_statement "$FILE" 4)
        se=$(get_statement "$FILE" 5)
        sf=$(get_statement "$FILE" 6)

        local sa_x sb_x sc_x sd_x se_x sf_x
        sa_x=$(substitute_params "$sa" "decode('$hex', 'hex')")
        sb_x=$(substitute_params "$sb" "decode('$hex', 'hex')" ":'_ca'::timestamptz")
        sc_x=$(substitute_params "$sc" ":_tx_id" ":'_ca'::timestamptz")
        sd_x=$(substitute_params "$sd" ":_tx_id" ":'_ca'::timestamptz")
        se_x=$(substitute_params "$se" ":_tx_id" ":'_ca'::timestamptz")
        sf_x=$(substitute_params "$sf" ":_tx_id" ":'_ca'::timestamptz")

        psql_pipe <<SQL
\\echo --- A: hash → (ledger_sequence, created_at)
$(explain_wrap "$sa_x")

\\echo --- (capture state for B–F)
SELECT ledger_sequence AS _ls, created_at AS _ca
FROM transaction_hash_index
WHERE hash = decode('$hex', 'hex')
\\gset

SELECT id AS _tx_id
FROM transactions
WHERE hash = decode('$hex', 'hex')
  AND created_at = :'_ca'::timestamptz
\\gset

\\echo --- B: header
$(explain_wrap "$sb_x")

\\echo --- C: operations
$(explain_wrap "$sc_x")

\\echo --- D: participants
$(explain_wrap "$sd_x")

\\echo --- E: soroban events appearance index
$(explain_wrap "$se_x")

\\echo --- F: soroban invocations appearance index
$(explain_wrap "$sf_x")
SQL
        ;;

    # -------------------------------------------------------------------------
    04)
        echo "=== E04: GET /ledgers ==="
        FILE="$QUERY_DIR/04_get_ledgers_list.sql"
        SUB=$(substitute_params "$(<"$FILE")" \
            "50" "NULL::timestamptz" "NULL::bigint")
        explain_wrap "$SUB" | psql_pipe
        ;;

    # -------------------------------------------------------------------------
    05)
        echo "=== E05: GET /ledgers/:sequence ==="
        local seq
        seq=$(psql_oneshot "SELECT MAX(sequence) FROM ledgers;")
        require_value "$seq" "ledgers" || return 1
        echo "  sequence = $seq"
        FILE="$QUERY_DIR/05_get_ledgers_by_sequence.sql"
        SUB=$(substitute_params "$(<"$FILE")" "$seq")
        explain_wrap "$SUB" | psql_pipe
        ;;

    # -------------------------------------------------------------------------
    06)
        echo "=== E06: GET /accounts/:account_id ==="
        local strkey
        strkey=$(psql_oneshot "SELECT account_id FROM accounts ORDER BY id DESC LIMIT 1;")
        require_value "$strkey" "accounts" || return 1
        echo "  account = $strkey"

        FILE="$QUERY_DIR/06_get_accounts_by_id.sql"
        local sa sb sa_x sb_x
        sa=$(get_statement "$FILE" 1)
        sb=$(get_statement "$FILE" 2)
        sa_x=$(substitute_params "$sa" "'$strkey'")
        sb_x=$(substitute_params "$sb" ":_acc_id")

        psql_pipe <<SQL
\\echo --- A: account header
$(explain_wrap "$sa_x")

\\echo --- (capture id for B)
SELECT id AS _acc_id FROM accounts WHERE account_id = '$strkey'
\\gset

\\echo --- B: balances
$(explain_wrap "$sb_x")
SQL
        ;;

    # -------------------------------------------------------------------------
    07)
        echo "=== E07: GET /accounts/:account_id/transactions ==="
        local strkey
        strkey=$(psql_oneshot "SELECT account_id FROM accounts ORDER BY id DESC LIMIT 1;")
        require_value "$strkey" "accounts" || return 1
        echo "  account = $strkey"
        FILE="$QUERY_DIR/07_get_accounts_transactions.sql"
        SUB=$(substitute_params "$(<"$FILE")" \
            "'$strkey'" "50" "NULL::timestamptz" "NULL::bigint")
        explain_wrap "$SUB" | psql_pipe
        ;;

    # -------------------------------------------------------------------------
    08)
        echo "=== E08: GET /assets ==="
        FILE="$QUERY_DIR/08_get_assets_list.sql"
        SUB=$(substitute_params "$(<"$FILE")" \
            "50" "NULL::int" "NULL::smallint" "NULL::text")
        explain_wrap "$SUB" | psql_pipe
        ;;

    # -------------------------------------------------------------------------
    09)
        echo "=== E09: GET /assets/:id ==="
        local aid
        aid=$(psql_oneshot "SELECT id FROM assets ORDER BY id DESC LIMIT 1;")
        require_value "$aid" "assets" || return 1
        echo "  asset id = $aid"
        FILE="$QUERY_DIR/09_get_assets_by_id.sql"
        SUB=$(substitute_params "$(<"$FILE")" "$aid")
        explain_wrap "$SUB" | psql_pipe
        ;;

    # -------------------------------------------------------------------------
    10)
        echo "=== E10: GET /assets/:id/transactions ==="
        FILE="$QUERY_DIR/10_get_assets_transactions.sql"
        local sa sb
        sa=$(get_statement "$FILE" 1)
        sb=$(get_statement "$FILE" 2)

        # Variant A: classic identity (asset_code + issuer)
        local aid_a
        aid_a=$(psql_oneshot "
            SELECT id FROM assets
            WHERE asset_code IS NOT NULL AND issuer_id IS NOT NULL
            ORDER BY id DESC LIMIT 1;
        ")
        if [[ -n "$aid_a" ]]; then
            echo "  variant A (classic identity), asset id = $aid_a"
            SUB=$(substitute_params "$sa" \
                "$aid_a" "50" "NULL::timestamptz" "NULL::bigint")
            explain_wrap "$SUB" | psql_pipe
        else
            echo "  no classic-identity asset; skipping variant A"
        fi

        # Variant B: contract identity
        local aid_b
        aid_b=$(psql_oneshot "
            SELECT id FROM assets
            WHERE contract_id IS NOT NULL
            ORDER BY id DESC LIMIT 1;
        ")
        if [[ -n "$aid_b" ]]; then
            echo "  variant B (contract identity), asset id = $aid_b"
            SUB=$(substitute_params "$sb" \
                "$aid_b" "50" "NULL::timestamptz" "NULL::bigint")
            explain_wrap "$SUB" | psql_pipe
        else
            echo "  no contract-identity asset; skipping variant B"
        fi
        ;;

    # -------------------------------------------------------------------------
    11)
        echo "=== E11: GET /contracts/:contract_id ==="
        local strkey
        strkey=$(psql_oneshot "SELECT contract_id FROM soroban_contracts ORDER BY id DESC LIMIT 1;")
        require_value "$strkey" "soroban_contracts" || return 1
        echo "  contract = $strkey"

        FILE="$QUERY_DIR/11_get_contracts_by_id.sql"
        local sa sb sa_x sb_x
        sa=$(get_statement "$FILE" 1)
        sb=$(get_statement "$FILE" 2)
        sa_x=$(substitute_params "$sa" "'$strkey'")
        sb_x=$(substitute_params "$sb" ":_ctr_pk" "'7 days'::interval")

        psql_pipe <<SQL
\\echo --- A: contract header
$(explain_wrap "$sa_x")

\\echo --- (capture contract pk)
SELECT id AS _ctr_pk FROM soroban_contracts WHERE contract_id = '$strkey'
\\gset

\\echo --- B: stats (window: 7 days)
$(explain_wrap "$sb_x")
SQL
        ;;

    # -------------------------------------------------------------------------
    12)
        echo "=== E12: GET /contracts/:contract_id/interface ==="
        local strkey
        strkey=$(psql_oneshot "SELECT contract_id FROM soroban_contracts ORDER BY id DESC LIMIT 1;")
        require_value "$strkey" "soroban_contracts" || return 1
        echo "  contract = $strkey"
        FILE="$QUERY_DIR/12_get_contracts_interface.sql"
        SUB=$(substitute_params "$(<"$FILE")" "'$strkey'")
        explain_wrap "$SUB" | psql_pipe
        ;;

    # -------------------------------------------------------------------------
    13)
        echo "=== E13: GET /contracts/:contract_id/invocations ==="
        local strkey
        strkey=$(psql_oneshot "SELECT contract_id FROM soroban_contracts ORDER BY id DESC LIMIT 1;")
        require_value "$strkey" "soroban_contracts" || return 1
        echo "  contract = $strkey"
        FILE="$QUERY_DIR/13_get_contracts_invocations.sql"
        SUB=$(substitute_params "$(<"$FILE")" \
            "'$strkey'" "50" "NULL::bigint" "NULL::bigint" "NULL::timestamptz")
        explain_wrap "$SUB" | psql_pipe
        ;;

    # -------------------------------------------------------------------------
    14)
        echo "=== E14: GET /contracts/:contract_id/events ==="
        local strkey
        strkey=$(psql_oneshot "SELECT contract_id FROM soroban_contracts ORDER BY id DESC LIMIT 1;")
        require_value "$strkey" "soroban_contracts" || return 1
        echo "  contract = $strkey"
        FILE="$QUERY_DIR/14_get_contracts_events.sql"
        SUB=$(substitute_params "$(<"$FILE")" \
            "'$strkey'" "50" "NULL::bigint" "NULL::bigint" "NULL::timestamptz")
        explain_wrap "$SUB" | psql_pipe
        ;;

    # -------------------------------------------------------------------------
    15)
        echo "=== E15: GET /nfts ==="
        FILE="$QUERY_DIR/15_get_nfts_list.sql"
        SUB=$(substitute_params "$(<"$FILE")" \
            "50" "NULL::int" "NULL::varchar" "NULL::varchar" "NULL::text")
        explain_wrap "$SUB" | psql_pipe
        ;;

    # -------------------------------------------------------------------------
    16)
        echo "=== E16: GET /nfts/:id ==="
        local nid
        nid=$(psql_oneshot "SELECT id FROM nfts ORDER BY id DESC LIMIT 1;")
        require_value "$nid" "nfts" || return 1
        echo "  nft id = $nid"
        FILE="$QUERY_DIR/16_get_nfts_by_id.sql"
        SUB=$(substitute_params "$(<"$FILE")" "$nid")
        explain_wrap "$SUB" | psql_pipe
        ;;

    # -------------------------------------------------------------------------
    17)
        echo "=== E17: GET /nfts/:id/transfers ==="
        local nid
        nid=$(psql_oneshot "SELECT id FROM nfts ORDER BY id DESC LIMIT 1;")
        require_value "$nid" "nfts" || return 1
        echo "  nft id = $nid"
        FILE="$QUERY_DIR/17_get_nfts_transfers.sql"
        SUB=$(substitute_params "$(<"$FILE")" \
            "$nid" "50" "NULL::timestamptz" "NULL::bigint" "NULL::smallint")
        explain_wrap "$SUB" | psql_pipe
        ;;

    # -------------------------------------------------------------------------
    18)
        echo "=== E18: GET /liquidity-pools ==="
        FILE="$QUERY_DIR/18_get_liquidity_pools_list.sql"
        SUB=$(substitute_params "$(<"$FILE")" \
            "50" "NULL::bigint" "NULL::bytea" \
            "NULL::varchar" "NULL::varchar" "NULL::varchar" "NULL::varchar" \
            "NULL::numeric" "'1 day'::interval")
        explain_wrap "$SUB" | psql_pipe
        ;;

    # -------------------------------------------------------------------------
    19)
        echo "=== E19: GET /liquidity-pools/:id ==="
        local pool_hex
        pool_hex=$(psql_oneshot "
            SELECT encode(pool_id, 'hex') FROM liquidity_pools
            ORDER BY created_at_ledger DESC LIMIT 1;
        ")
        require_value "$pool_hex" "liquidity_pools" || return 1
        echo "  pool = $pool_hex"
        FILE="$QUERY_DIR/19_get_liquidity_pools_by_id.sql"
        SUB=$(substitute_params "$(<"$FILE")" \
            "decode('$pool_hex', 'hex')" "'7 days'::interval")
        explain_wrap "$SUB" | psql_pipe
        ;;

    # -------------------------------------------------------------------------
    20)
        echo "=== E20: GET /liquidity-pools/:id/transactions ==="
        local pool_hex
        pool_hex=$(psql_oneshot "
            SELECT encode(pool_id, 'hex') FROM liquidity_pools
            ORDER BY created_at_ledger DESC LIMIT 1;
        ")
        require_value "$pool_hex" "liquidity_pools" || return 1
        echo "  pool = $pool_hex"
        FILE="$QUERY_DIR/20_get_liquidity_pools_transactions.sql"
        SUB=$(substitute_params "$(<"$FILE")" \
            "decode('$pool_hex', 'hex')" "50" "NULL::timestamptz" "NULL::bigint")
        explain_wrap "$SUB" | psql_pipe
        ;;

    # -------------------------------------------------------------------------
    21)
        echo "=== E21: GET /liquidity-pools/:id/chart ==="
        local pool_hex
        pool_hex=$(psql_oneshot "
            SELECT encode(pool_id, 'hex') FROM liquidity_pools
            ORDER BY created_at_ledger DESC LIMIT 1;
        ")
        require_value "$pool_hex" "liquidity_pools" || return 1
        echo "  pool = $pool_hex (interval: day, range: last 30 days)"
        FILE="$QUERY_DIR/21_get_liquidity_pools_chart.sql"
        SUB=$(substitute_params "$(<"$FILE")" \
            "decode('$pool_hex', 'hex')" "'day'" \
            "(NOW() - INTERVAL '30 days')" "NOW()")
        explain_wrap "$SUB" | psql_pipe
        ;;

    # -------------------------------------------------------------------------
    22)
        echo "=== E22: GET /search ==="
        # Use a 4-char prefix of a real account StrKey as the test query,
        # routing to the StrKey-prefix CTEs (account + contract).
        local prefix
        prefix=$(psql_oneshot "
            SELECT substring(account_id, 1, 4) FROM accounts
            ORDER BY id DESC LIMIT 1;
        ")
        require_value "$prefix" "accounts" || return 1
        echo "  query = $prefix (StrKey prefix; pool/tx CTEs disabled)"
        FILE="$QUERY_DIR/22_get_search.sql"
        SUB=$(substitute_params "$(<"$FILE")" \
            "'$prefix'" "NULL::bytea" "'$prefix'" "10" \
            "FALSE" "TRUE" "TRUE" "TRUE" "TRUE" "FALSE")
        explain_wrap "$SUB" | psql_pipe
        ;;

    # -------------------------------------------------------------------------
    23)
        echo "=== E23: GET /liquidity-pools/:id/participants ==="
        # Pick a pool that actually has at least one position with shares > 0
        # so the keyset returns something meaningful.
        local pool_hex
        pool_hex=$(psql_oneshot "
            SELECT encode(lp.pool_id, 'hex')
            FROM liquidity_pools lp
            JOIN lp_positions p ON p.pool_id = lp.pool_id
            WHERE p.shares > 0
            ORDER BY lp.created_at_ledger DESC
            LIMIT 1;
        ")
        require_value "$pool_hex" "lp_positions (with shares > 0)" || return 1
        echo "  pool = $pool_hex"
        FILE="$QUERY_DIR/23_get_liquidity_pools_participants.sql"
        SUB=$(substitute_params "$(<"$FILE")" \
            "decode('$pool_hex', 'hex')" "50" "NULL::numeric" "NULL::bigint")
        explain_wrap "$SUB" | psql_pipe
        ;;

    # -------------------------------------------------------------------------
    *)
        echo "unknown id: $id" >&2
        return 1
        ;;
    esac
}

# =============================================================================
# Dispatch
# =============================================================================
if [[ "$ID" == "all" ]]; then
    for i in 01 02 03 04 05 06 07 08 09 10 11 12 13 14 15 16 17 18 19 20 21 22 23; do
        echo
        run_one "$i" || echo "  -> failed (id=$i)"
    done
else
    run_one "$ID"
fi
