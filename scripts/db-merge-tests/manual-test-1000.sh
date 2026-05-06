#!/usr/bin/env bash
# Manual end-to-end test of db-merge against real Stellar mainnet data.
#
# Range: 1000 ledgers starting from 62_016_000, split into 4× 250-ledger
# snapshots (A/B/C/D) chronologically. Single laptop slot reused 4 times
# (option A — sequential). Truth = full 1000-ledger sequential backfill.
#
# Run: bash scripts/db-merge-tests/manual-test-1000.sh
# Env override: START=<sequence> COUNT=<per-snapshot> ./manual-test-1000.sh
#
# Leaves state intact at end for psql inspection. Manual teardown when done.

set -euo pipefail
LIB="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/_lib.sh"
. "$LIB"

# Backfill-runner indexer needs STELLAR_NETWORK_PASSPHRASE for SAC
# contract_id derivation — sourced from repo .env (npm scripts get this
# automatically; bash doesn't).
if [[ -f "$REPO_ROOT/.env" ]]; then
    set -a
    . "$REPO_ROOT/.env"
    set +a
fi

START="${START:-62016000}"
COUNT="${COUNT:-250}"

A_START=$START                  ; A_END=$((A_START + COUNT - 1))
B_START=$((A_END + 1))          ; B_END=$((B_START + COUNT - 1))
C_START=$((B_END + 1))          ; C_END=$((C_START + COUNT - 1))
D_START=$((C_END + 1))          ; D_END=$((D_START + COUNT - 1))
TRUTH_END=$D_END

BACKFILL_BIN="$REPO_ROOT/target/release/backfill-runner"
DUMPS_DIR="$REPO_ROOT/.temp/db-merge-fixtures"
mkdir -p "$DUMPS_DIR"

# Optional: pre-hydrated S3 partition cache. If a directory matching
# `<temp-dir>/FC4DB5FF--XXXXXXX-YYYYYYY/` is dropped here as hardlinks
# (or downloaded once), subsequent backfill-runner invocations skip the
# `aws s3 sync` step entirely (`sync` becomes a no-op when files match).
# Backfill-runner deletes the partition dir after success — so we
# re-hardlink before every invocation.
BENCH_CACHE_DIR="$REPO_ROOT/crates/backfill-bench/.temp"
PARTITION_NAME="FC4DB5FF--62016000-62079999"  # covers START..D_END

# Re-hydrate a backfill-runner temp-dir from bench cache (no-op if cache missing).
hydrate_temp_dir() {
    local temp_dir="$1"
    local src="$BENCH_CACHE_DIR/$PARTITION_NAME"
    local dst="$temp_dir/$PARTITION_NAME"
    if [[ -d "$src" && ! -d "$dst" ]]; then
        mkdir -p "$temp_dir"
        cp -al "$src" "$dst"
    fi
}

ts() { date '+%H:%M:%S'; }
say() { echo; echo "[$(ts)] >>> $*"; }

say "build (release for backfill speed)"
cd "$REPO_ROOT"
cargo build --release -p db-merge -p backfill-runner

# Re-point DB_MERGE_BIN to release build for consistency
DB_MERGE_BIN="$REPO_ROOT/target/release/db-merge"

say "ranges"
printf "  truth   = [%d..%d]  (%d ledgers)\n" "$START" "$TRUTH_END" $((TRUTH_END - START + 1))
printf "  laptop A = [%d..%d]  (%d ledgers)\n" "$A_START" "$A_END" "$COUNT"
printf "  laptop B = [%d..%d]  (%d ledgers)\n" "$B_START" "$B_END" "$COUNT"
printf "  laptop C = [%d..%d]  (%d ledgers)\n" "$C_START" "$C_END" "$COUNT"
printf "  laptop D = [%d..%d]  (%d ledgers)\n" "$D_START" "$D_END" "$COUNT"

say "wipe + recreate truth, merge, snapshot-source (laptop-a reset per cycle below)"
# WIPES any stale data from prior runs — without this, truth backfill
# hits ON CONFLICT and merge preflight may reject on the chronological
# gate. Volume drop is the only sufficient reset (truncate leaves
# sequence state + partition children behind).
for s in postgres-truth postgres-merge postgres-snapshot-source postgres-laptop-a postgres-laptop-b; do
    reset_db "$s"
done

say "migrate + partition truth, laptop-a, merge"
for s in truth laptop-a merge; do
    var="URL_$(echo "${s//-/_}" | tr '[:lower:]' '[:upper:]')"
    migrate "${!var}"
    create_partitions "${!var}"
done

say "kick off truth backfill in background — 1000 ledgers, slowest leg"
hydrate_temp_dir "$REPO_ROOT/.temp/backfill-runner/truth"
"$BACKFILL_BIN" --database-url "$URL_TRUTH" --temp-dir "$REPO_ROOT/.temp/backfill-runner/truth" \
    run --start $START --end $TRUTH_END > "$REPO_ROOT/.temp/truth.log" 2>&1 &
TRUTH_PID=$!
echo "  truth backfill PID=$TRUTH_PID, log: .temp/truth.log"

backfill_one() {
    local letter="$1" lo="$2" hi="$3"
    say "laptop ${letter}: reset + migrate + partition + backfill [${lo}..${hi}]"
    reset_db postgres-laptop-a
    migrate "$URL_LAPTOP_A"
    create_partitions "$URL_LAPTOP_A"
    hydrate_temp_dir "$REPO_ROOT/.temp/backfill-runner/laptop"
    "$BACKFILL_BIN" --database-url "$URL_LAPTOP_A" \
        --temp-dir "$REPO_ROOT/.temp/backfill-runner/laptop" \
        run --start "$lo" --end "$hi"
    say "laptop ${letter}: dump"
    pg_dump --format=custom -f "$DUMPS_DIR/laptop-${letter}.dump" "$URL_LAPTOP_A"
    ls -lh "$DUMPS_DIR/laptop-${letter}.dump" | awk '{print "  size:", $5}'
}

backfill_one A "$A_START" "$A_END"
backfill_one B "$B_START" "$B_END"
backfill_one C "$C_START" "$C_END"
backfill_one D "$D_START" "$D_END"

say "wait for truth backfill (PID=$TRUTH_PID)"
wait "$TRUTH_PID"
echo "  truth done"

say "merge — 4 ingests in chronological order (A → D)"
for letter in A B C D; do
    say "ingest laptop-${letter}"
    "$DB_MERGE_BIN" ingest "$DUMPS_DIR/laptop-${letter}.dump" \
        --target-url "$URL_MERGE" --snapshot-source-url "$URL_SNAPSHOT_SOURCE"
done

say "finalize merge target"
"$DB_MERGE_BIN" finalize --target-url "$URL_MERGE"

say "finalize truth (rebuilds nfts.current_owner_* from events — same as merge)"
"$DB_MERGE_BIN" finalize --target-url "$URL_TRUTH"

say "DIFF — truth vs merge"
"$DB_MERGE_BIN" diff --left "$URL_TRUTH" --right "$URL_MERGE"
DIFF_RC=$?

say "DONE"
echo "  truth log:    .temp/truth.log"
echo "  dumps:        $DUMPS_DIR/laptop-{A,B,C,D}.dump"
echo "  pre-merge bk: .temp/db-merge-backups/"
echo "  truth URL:    $URL_TRUTH"
echo "  merge URL:    $URL_MERGE"
echo
if [[ $DIFF_RC -eq 0 ]]; then
    echo "  RESULT: PASS — truth and merge are logically identical (17/17 tables)"
else
    echo "  RESULT: FAIL — diff returned $DIFF_RC; investigate mismatched tables above"
fi
echo
echo "  state left intact for psql inspection. Teardown when done:"
echo "    docker compose --profile db-merge stop \\"
echo "      postgres-truth postgres-laptop-a postgres-laptop-b postgres-merge postgres-snapshot-source"
echo "    docker compose --profile db-merge rm -f \\"
echo "      postgres-truth postgres-laptop-a postgres-laptop-b postgres-merge postgres-snapshot-source"
echo "    docker volume rm \\"
echo "      sorban-block-explorer_pgdata-{truth,laptop-a,laptop-b,merge,snapshot-source}"

exit $DIFF_RC
