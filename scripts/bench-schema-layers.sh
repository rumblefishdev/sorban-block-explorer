#!/usr/bin/env bash
# Benchmark: measure impact of FK, UNIQUE, and index layers on write performance.
# Runs 4 rounds on the same ledger range, dropping one layer at a time.
#
# Usage: ./scripts/bench-schema-layers.sh
#
# Prerequisites:
#   - PostgreSQL running with the schema applied
#   - backfill-bench built (cargo build --release -p backfill-bench)
#   - Target ledger range NOT in database (script cleans up between rounds)

set -euo pipefail

DB_URL="postgres://postgres:postgres@127.0.0.1:5432/soroban_block_explorer"
START=62016000
END=62016099
BENCH="cargo run --release -p backfill-bench --"

psql_cmd() {
    psql "$DB_URL" -q -c "$1"
}

clean_range() {
    echo "  Cleaning ledger range ${START}-${END}..."
    # Delete child tables first (FK may be dropped, so CASCADE won't work)
    psql_cmd "DELETE FROM soroban_invocations WHERE ledger_sequence BETWEEN ${START} AND ${END};"
    psql_cmd "DELETE FROM soroban_events WHERE ledger_sequence BETWEEN ${START} AND ${END};"
    psql_cmd "DELETE FROM operations WHERE transaction_id IN (SELECT id FROM transactions WHERE ledger_sequence BETWEEN ${START} AND ${END});"
    psql_cmd "DELETE FROM transactions WHERE ledger_sequence BETWEEN ${START} AND ${END};"
    psql_cmd "DELETE FROM ledgers WHERE sequence BETWEEN ${START} AND ${END};"
}

run_bench() {
    local label="$1"
    echo ""
    echo "=========================================="
    echo "  Round: ${label}"
    echo "=========================================="
    clean_range
    echo "  Running benchmark..."
    $BENCH --start "$START" --end "$END" --database-url "$DB_URL" 2>&1 | grep -E "(Avg per ledger|Index time|Indexed)"
}

# ── Round 1: Baseline ──────────────────────────────────────────────────
run_bench "BASELINE (all constraints + indexes)"

# ── Round 2: Drop FK constraints ───────────────────────────────────────
echo ""
echo "--- Dropping FK constraints ---"
psql_cmd "ALTER TABLE soroban_events DROP CONSTRAINT soroban_events_transaction_id_fkey;"
psql_cmd "ALTER TABLE soroban_events DROP CONSTRAINT soroban_events_contract_id_fkey;"
psql_cmd "ALTER TABLE operations DROP CONSTRAINT operations_transaction_id_fkey;"
psql_cmd "ALTER TABLE transactions DROP CONSTRAINT transactions_ledger_sequence_fkey;"
psql_cmd "ALTER TABLE soroban_invocations DROP CONSTRAINT soroban_invocations_transaction_id_fkey;"
psql_cmd "ALTER TABLE soroban_invocations DROP CONSTRAINT soroban_invocations_contract_id_fkey;"

run_bench "NO FK (UNIQUE + indexes remain)"

# ── Round 3: Also drop UNIQUE constraints ─────────────────────────────
echo ""
echo "--- Dropping UNIQUE constraints (events/operations/invocations) ---"
psql_cmd "ALTER TABLE soroban_events DROP CONSTRAINT uq_events_tx_index;"
psql_cmd "ALTER TABLE operations DROP CONSTRAINT uq_operations_tx_order;"
psql_cmd "ALTER TABLE soroban_invocations DROP CONSTRAINT uq_invocations_tx_index;"
# Note: transactions_hash_key kept — needed for ON CONFLICT (hash) in code

run_bench "NO FK + NO UNIQUE (indexes remain, tx hash kept)"

# ── Round 4: Also drop non-essential indexes ───────────────────────────
echo ""
echo "--- Dropping non-essential indexes ---"
# Events
psql_cmd "DROP INDEX IF EXISTS idx_events_topics;"
psql_cmd "DROP INDEX IF EXISTS idx_events_contract;"
psql_cmd "DROP INDEX IF EXISTS idx_events_tx;"
# Operations
psql_cmd "DROP INDEX IF EXISTS idx_operations_details;"
psql_cmd "DROP INDEX IF EXISTS idx_operations_source;"
psql_cmd "DROP INDEX IF EXISTS idx_operations_tx;"
# Invocations
psql_cmd "DROP INDEX IF EXISTS idx_invocations_contract;"
psql_cmd "DROP INDEX IF EXISTS idx_invocations_function;"
psql_cmd "DROP INDEX IF EXISTS idx_invocations_tx;"
# Transactions
psql_cmd "DROP INDEX IF EXISTS idx_source;"
psql_cmd "DROP INDEX IF EXISTS idx_ledger;"

run_bench "NO FK + NO UNIQUE + NO INDEXES (PK only)"

# ── Restore everything ─────────────────────────────────────────────────
echo ""
echo "=========================================="
echo "  Restoring all constraints and indexes"
echo "=========================================="
clean_range

# Restore UNIQUE constraints
psql_cmd "ALTER TABLE soroban_events ADD CONSTRAINT uq_events_tx_index UNIQUE (transaction_id, event_index, created_at);"
psql_cmd "ALTER TABLE operations ADD CONSTRAINT uq_operations_tx_order UNIQUE (transaction_id, application_order);"
psql_cmd "ALTER TABLE soroban_invocations ADD CONSTRAINT uq_invocations_tx_index UNIQUE (transaction_id, invocation_index, created_at);"
# transactions_hash_key was never dropped — nothing to restore

# Restore FK constraints
psql_cmd "ALTER TABLE transactions ADD CONSTRAINT transactions_ledger_sequence_fkey FOREIGN KEY (ledger_sequence) REFERENCES ledgers(sequence);"
psql_cmd "ALTER TABLE operations ADD CONSTRAINT operations_transaction_id_fkey FOREIGN KEY (transaction_id) REFERENCES transactions(id) ON DELETE CASCADE;"
psql_cmd "ALTER TABLE soroban_events ADD CONSTRAINT soroban_events_transaction_id_fkey FOREIGN KEY (transaction_id) REFERENCES transactions(id) ON DELETE CASCADE;"
psql_cmd "ALTER TABLE soroban_events ADD CONSTRAINT soroban_events_contract_id_fkey FOREIGN KEY (contract_id) REFERENCES soroban_contracts(contract_id);"
psql_cmd "ALTER TABLE soroban_invocations ADD CONSTRAINT soroban_invocations_transaction_id_fkey FOREIGN KEY (transaction_id) REFERENCES transactions(id) ON DELETE CASCADE;"
psql_cmd "ALTER TABLE soroban_invocations ADD CONSTRAINT soroban_invocations_contract_id_fkey FOREIGN KEY (contract_id) REFERENCES soroban_contracts(contract_id);"

# Restore indexes
psql_cmd "CREATE INDEX idx_events_topics ON soroban_events USING GIN (topics);"
psql_cmd "CREATE INDEX idx_events_contract ON soroban_events (contract_id, created_at DESC);"
psql_cmd "CREATE INDEX idx_events_tx ON soroban_events (transaction_id);"
psql_cmd "CREATE INDEX idx_operations_details ON operations USING GIN (details);"
psql_cmd "CREATE INDEX idx_operations_source ON operations (source_account);"
psql_cmd "CREATE INDEX idx_operations_tx ON operations (transaction_id);"
psql_cmd "CREATE INDEX idx_invocations_contract ON soroban_invocations (contract_id, created_at DESC);"
psql_cmd "CREATE INDEX idx_invocations_function ON soroban_invocations (contract_id, function_name);"
psql_cmd "CREATE INDEX idx_invocations_tx ON soroban_invocations (transaction_id);"
psql_cmd "CREATE INDEX idx_source ON transactions (source_account, created_at DESC);"
psql_cmd "CREATE INDEX idx_ledger ON transactions (ledger_sequence);"

echo "  All constraints and indexes restored."
echo ""
echo "Done. Compare 'Avg per ledger' across rounds."
