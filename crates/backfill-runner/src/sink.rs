//! ClickHouse write-path sink for the backfill runner.
//!
//! Task 0205 / ADR 0044 introduced a target-agnostic `Sink`; task 0244
//! collapsed it to ClickHouse-only when Postgres was removed. Task 0206's
//! **partition-writer lifecycle** (`open_partition` → `write_ledger × N` →
//! `commit` / `abort`) drives a [`db_clickhouse::persist::PartitionWriter`]
//! across the partition so the 14 server-side INSERTs only open once and end
//! once — see that module's docs for why per-ledger inserts are wrong for CH
//! (parts explosion, merger overwhelmed).
//!
//! The legacy [`Sink::persist_ledger`] method is kept as a thin wrapper
//! (`open` → `write` → `commit`) for tests and any caller that wants
//! per-ledger semantics. Production backfill goes through the lifecycle on
//! `ingest.rs::index_partition`.

use std::collections::HashSet;

use clickhouse::Client as ClickhouseClient;
use serde::Deserialize;
use stellar_xdr::LedgerCloseMeta;
use tracing::{info, warn};

use crate::error::BackfillError;

/// ClickHouse write-path handle wired up at startup. Exactly one `Sink`
/// exists per process; the runner passes `&Sink` down — no clones needed.
pub struct Sink {
    client: ClickhouseClient,
    lp_amounts_only: bool,
}

/// Row shape for the resume / status query against ClickHouse. Private
/// to this module — callers see `HashSet<u32>`.
#[derive(Debug, Deserialize, clickhouse::Row)]
struct LedgerSeqRow {
    sequence: i64,
}

impl Sink {
    /// Wrap a ClickHouse client as the write-path sink.
    pub fn new(client: ClickhouseClient) -> Self {
        Self {
            client,
            lp_amounts_only: false,
        }
    }

    /// Switch the whole process to the **targeted write** of task 0279: parse
    /// as usual, but persist ONLY `lp_operation_amounts`.
    ///
    /// This is what makes a historical re-parse for one new derived table
    /// additive. A normal run re-emits every table, which rewrites the 12
    /// Tier-1 columns that cannot survive parallel `ReplacingMergeTree`
    /// collapse (`docs/backfills.md` §3) and so owes a `repair-tier1` pass
    /// afterwards; this writes one table that has no such column.
    ///
    /// Consequence to plan for: no `ledgers` commit marker is written, so
    /// resume cannot read progress from the DB — see
    /// [`db_clickhouse::persist::PartitionWriter::write_lp_amounts_only`].
    pub fn with_lp_amounts_only(mut self, on: bool) -> Self {
        self.lp_amounts_only = on;
        self
    }

    /// Is this process running the 0279 targeted write?
    pub fn lp_amounts_only(&self) -> bool {
        self.lp_amounts_only
    }

    /// Borrow the underlying ClickHouse client (backfill passes read it
    /// directly for their one-shot maintenance queries).
    pub fn client(&self) -> &ClickhouseClient {
        &self.client
    }

    /// Confirm the store is reachable (`SELECT 1`).
    ///
    /// A failure here is a config / environment error, so the caller panics
    /// on it. We still return `Result` so the panic site stays in `run.rs`.
    pub async fn preflight(&self) -> Result<(), BackfillError> {
        let _ = self.client.query("SELECT 1").fetch_one::<u8>().await?;
        Ok(())
    }

    /// Load sequences already present in the `ledgers` table within
    /// `[start, end]`. Resume + status both feed off this single batch
    /// query.
    pub async fn load_completed(
        &self,
        start: u32,
        end: u32,
    ) -> Result<HashSet<u32>, BackfillError> {
        let rows: Vec<i64> = self
            .client
            .query("SELECT sequence FROM ledgers WHERE sequence BETWEEN ? AND ?")
            .bind(i64::from(start))
            .bind(i64::from(end))
            .fetch_all::<LedgerSeqRow>()
            .await?
            .into_iter()
            .map(|r| r.sequence)
            .collect();
        // `sequence` is i64 in the CH schema but ledger sequences are
        // u32-bounded by Stellar protocol. The SQL `BETWEEN start AND end`
        // already constrains the range, but defend against bogus /
        // manually-inserted rows via `try_from` and warn on anything that
        // doesn't fit. A silent `as u32` would wrap negatives / overflows.
        let set: HashSet<u32> = rows
            .into_iter()
            .filter_map(|s| match u32::try_from(s) {
                Ok(v) => Some(v),
                Err(_) => {
                    warn!(
                        value = s,
                        "skipping out-of-range sequence from clickhouse ledgers"
                    );
                    None
                }
            })
            .collect();
        info!(
            start,
            end,
            completed = set.len(),
            total = u64::from(end) - u64::from(start) + 1,
            target = "clickhouse",
            "resume state loaded"
        );
        Ok(set)
    }

    /// Open a partition writer handle — a long-lived
    /// [`db_clickhouse::persist::PartitionWriter`] holding one `Insert<RowT>`
    /// per table across the partition.
    pub fn open_partition(&self) -> PartitionWriterHandle {
        PartitionWriterHandle {
            writer: db_clickhouse::persist::PartitionWriter::open(self.client.clone()),
            lp_amounts_only: self.lp_amounts_only,
        }
    }

    /// Parse + persist a single ledger.
    ///
    /// Kept for legacy / per-ledger callers (tests and any direct
    /// invocation that doesn't want to drive the lifecycle). Wraps
    /// `open_partition` → `write_ledger` → `commit`.
    ///
    /// Production backfill (`ingest.rs::index_partition`) drives the
    /// lifecycle explicitly across the whole partition — never calls
    /// this method.
    #[allow(dead_code)]
    pub async fn persist_ledger(&self, meta: &LedgerCloseMeta) -> Result<(), BackfillError> {
        let mut handle = self.open_partition();
        if let Err(err) = handle.write_ledger(meta).await {
            handle.abort().await;
            return Err(err);
        }
        handle.commit().await
    }
}

/// Lifecycle handle for one backfill partition's writes. Owns a
/// [`db_clickhouse::persist::PartitionWriter`] — the 14-handle insert
/// lifecycle described in `db-clickhouse/src/persist/writer.rs`.
pub struct PartitionWriterHandle {
    writer: db_clickhouse::persist::PartitionWriter,
    /// Task 0279 targeted write — see [`Sink::with_lp_amounts_only`].
    lp_amounts_only: bool,
}

impl PartitionWriterHandle {
    pub async fn write_ledger(&mut self, meta: &LedgerCloseMeta) -> Result<(), BackfillError> {
        let lp_amounts_only = self.lp_amounts_only;
        let pw = &mut self.writer;
        {
            let parsed = indexer::handler::process::parse_ledger(meta);
            // ADR 0051 — re-key contract-held type-0/1 balances onto their
            // wrapped classic/native asset_id, same as the live indexer and
            // RPC `balance-seed`. The fetch guards on empty balances, so
            // ledgers with no SAC/token balances skip the query.
            // ponytail: per-ledger query on the small `asset_sac` table; the
            // `Run` path is the rarely-used heavy fallback, so no cross-ledger
            // cache. Add one if a full reprocess ever makes this hot.
            //
            // Skipped entirely under the 0279 targeted write: the map only
            // re-keys BALANCE rows, which that mode does not persist, so the
            // query would be a per-ledger round-trip bought for nothing —
            // 13.16M of them across the run.
            let sac_classic = if lp_amounts_only {
                std::collections::HashMap::new()
            } else {
                db_clickhouse::persist::fetch_sac_classic_map(
                    pw.client(),
                    &parsed.soroban_token_balances,
                )
                .await?
            };
            // Task 0220 — switch to the `_with_sac_overrides` entry
            // point so the CH writer flips `is_sac=true,
            // contract_type=Token` on pre-existing SAC skeleton
            // rows via the forward-derived `ParseOutput.sac_overrides`
            // list. The legacy `stage::prepare` is kept as a
            // backwards-compat shim with empty overrides; this is
            // the production wire-up the PR #186 description called
            // out as a follow-up.
            let staged = db_clickhouse::persist::stage::prepare_with_sac_overrides(
                &db_clickhouse::persist::stage::StageInputs {
                    ledger: &parsed.ledger,
                    transactions: &parsed.transactions,
                    operations: &parsed.operations,
                    events: &parsed.events,
                    invocations: &parsed.invocations,
                    contract_interfaces: &parsed.contract_interfaces,
                    contract_deployments: &parsed.contract_deployments,
                    account_states: &parsed.account_states,
                    liquidity_pools: &parsed.liquidity_pools,
                    pool_snapshots: &parsed.pool_snapshots,
                    assets: &parsed.assets,
                    nfts: &parsed.nfts,
                    nft_events: &parsed.nft_events,
                    lp_positions: &parsed.lp_positions,
                    contract_metadata_writes: &parsed.contract_metadata_writes,
                    // Task 0331 — backfill reprocesses ledger ContractData
                    // changes through the shared `process.rs`, so this is
                    // populated for free: the historical-balance seed pass is
                    // the existing backfill, not a new crate. (TTL-archived
                    // entries never re-emitted in-window stay absent — the
                    // open caveat.)
                    soroban_token_balances: &parsed.soroban_token_balances,
                    // Task 0374 step 7 — pool state rides the shared
                    // `process.rs` extraction exactly like token balances
                    // above, so the historical re-parse emits
                    // `pool_state_changes` / `pool_instance_state` for free.
                    pool_family_writes: &parsed.pool_family_writes,
                    sac_classic: &sac_classic,
                    sac_overrides: &parsed.sac_overrides,
                    // Task 0283 live G1/G9 are for the live indexer path only.
                    // Backfill stays as-is (empty maps = pre-0283 behaviour):
                    // historical cross-ledger verdicts are reconstructed by
                    // the batch `ch-maint contract-type-rebuild` + one-shot
                    // `nft-reclassify`, not inline.
                    prior_wasm_verdicts: &std::collections::HashMap::new(),
                    prior_contract_verdicts: &std::collections::HashMap::new(),
                    // Task 0320 live WASM-upgrade rewrite is live-indexer-only;
                    // the backfill recovers stale hashes via the dedicated
                    // `wasm-upgrade-backfill` pass, so pass an empty map here.
                    prior_contract_rows: &std::collections::HashMap::new(),
                },
            )?;
            if lp_amounts_only {
                pw.write_lp_amounts_only(&staged).await?;
            } else {
                pw.write_ledger(staged).await?;
            }
            Ok(())
        }
    }

    /// End every open insert. Mid-partition failure must call
    /// [`Self::abort`] instead.
    pub async fn commit(self) -> Result<(), BackfillError> {
        self.writer.commit().await?;
        Ok(())
    }

    /// Abandon the partition: drops in-flight insert handles without ending
    /// them, ensuring resume finds no `ledgers` rows for this partition's
    /// range and re-does it cleanly.
    pub async fn abort(self) {
        self.writer.abort().await
    }
}

#[cfg(test)]
mod tests {
    //! ClickHouse-flavored Sink tests. Gated on `CLICKHOUSE_URL` so
    //! `cargo test -p backfill-runner` stays green in CI without a
    //! ClickHouse instance — mirrors the gating posture used by
    //! `db-clickhouse/tests/smoke.rs`.
    //!
    //! Run locally (against the `docker-compose up clickhouse` instance):
    //!
    //! ```bash
    //! CLICKHOUSE_URL=http://localhost:8123 \
    //!     cargo test -p backfill-runner --lib sink -- --test-threads=1
    //! ```
    use super::*;
    use db_clickhouse::{Config, apply_init_sql};

    /// Far above any realistic Soroban sequence — fits in u32 and i64,
    /// keeps fixtures out of the way of any real data on a shared CH.
    const TEST_BASE: u32 = 4_000_000_000;

    async fn build_sink() -> Option<Sink> {
        let url = std::env::var("CLICKHOUSE_URL").ok()?;
        let cfg = Config {
            url,
            ..Config::from_env()
        };
        let client = db_clickhouse::client(&cfg);
        // Make sure the schema is in place before the test runs the queries.
        // `apply_init_sql` is idempotent so calling it on every test run is
        // cheap and survives a fresh container.
        if let Err(err) = apply_init_sql(&client).await {
            eprintln!("CLICKHOUSE_URL set but apply_init_sql failed ({err}) — skipping");
            return None;
        }
        Some(Sink::new(client))
    }

    async fn ch_cleanup(client: &ClickhouseClient, start: u32, end: u32) {
        let _ = client
            .query("ALTER TABLE ledgers DELETE WHERE sequence BETWEEN ? AND ?")
            .bind(i64::from(start))
            .bind(i64::from(end))
            .with_setting("mutations_sync", "1")
            .execute()
            .await;
    }

    async fn ch_insert_ledger(client: &ClickhouseClient, seq: u32) {
        client
            .query(
                "INSERT INTO ledgers (sequence, hash, closed_at, protocol_version, transaction_count, base_fee) \
                 VALUES (?, unhex(?), now64(3), 22, 0, 100)",
            )
            .bind(i64::from(seq))
            // 32-byte hash unique per seq.
            .bind(format!("{:062x}{:02x}", 0u32, (seq & 0xff) as u8))
            .execute()
            .await
            .expect("insert fixture ledger");
    }

    #[tokio::test]
    async fn clickhouse_preflight_succeeds() {
        let Some(sink) = build_sink().await else {
            eprintln!("CLICKHOUSE_URL not set — skipping");
            return;
        };
        sink.preflight()
            .await
            .expect("CH preflight (SELECT 1) must succeed");
    }

    #[tokio::test]
    async fn clickhouse_load_completed_returns_only_in_range() {
        let Some(sink) = build_sink().await else {
            eprintln!("CLICKHOUSE_URL not set — skipping");
            return;
        };
        let client = sink.client();

        let below = TEST_BASE + 100;
        let a = TEST_BASE + 110;
        let b = TEST_BASE + 111;
        let c = TEST_BASE + 115;
        let above = TEST_BASE + 120;

        ch_cleanup(client, below, above).await;
        for seq in [below, a, b, c, above] {
            ch_insert_ledger(client, seq).await;
        }

        let got = sink.load_completed(a, c).await.expect("load_completed");
        let expected: HashSet<u32> = [a, b, c].into_iter().collect();
        assert_eq!(got, expected);

        ch_cleanup(client, below, above).await;
    }
}
