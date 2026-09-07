//! Partition-aligned streaming writer.
//!
//! Owns one long-lived [`clickhouse::insert::Insert<RowT>`] handle per
//! table, lazy-initialised on the first row destined for that table
//! within the partition. The lifecycle is:
//!
//! ```text
//! open()  →  write_ledger() × N  →  commit()
//!                                    └─ closes every non-ledger insert
//!                                       in FK order, THEN opens + ends
//!                                       the ledgers insert as a
//!                                       commit-marker.
//! ```
//!
//! ## Why one long-lived insert per table per partition
//!
//! ClickHouse `MergeTree` creates exactly one "part" per `INSERT`
//! statement. The writer streams into 18 non-ledger tables; `ledgers`
//! is opened once at commit as the commit marker (19 tables total).
//! With 18 streaming tables × 11 M ledgers the naive per-ledger
//! pattern produces ~200 M parts and trips
//! `parts_to_throw_insert = 3000` (per `(table, partition)`) after the
//! first ~3 k ledgers (≈ 0.03 % of an 11 M backfill). The merger
//! cannot fold parts faster than they're produced at parse-bound
//! throughput.
//!
//! Partition-aligned streaming inserts hold the request open across the
//! whole 64 k-ledger partition: ~172 partitions × 18 streaming tables
//! ≈ 3 100 `INSERT` statements over the entire 11 M-ledger backfill
//! (plus one `ledgers` INSERT per partition as the commit marker).
//! Well within the merger's comfort zone. The two new pending tables
//! (`nfts_pending`, `nft_ownership_pending`, task 0217 / 0220) only
//! open when the partition contains at least one `Other`-classified
//! NFT-candidate contract, so partitions with full classifier coverage
//! still see exactly the 16 prior inserts.
//!
//! Loopback transport is irrelevant here — server-side part economics
//! are the load-bearing constraint, not HTTP round-trips.
//!
//! ## Commit-marker pattern
//!
//! `ledgers` rows are buffered in [`PartitionWriter::ledger_rows`]
//! during `write_ledger`. At [`PartitionWriter::commit`] every other
//! table's `Insert::end()` is awaited first; only after every one
//! ack's does the `ledgers` insert open and end. Mid-partition failure
//! ⇒ no `ledgers` rows for that range ⇒ resume re-does the whole
//! partition. The orphan rows (if any) on the partial first attempt
//! dedupe under ReplacingMergeTree on the next background merge.
//!
//! ## Memory budget
//!
//! Each `clickhouse::Insert<T>` buffers `BUFFER_SIZE = 256 KiB` and
//! chunk-flushes when full. Up to 18 streaming inserts × 256 KiB ≈
//! 4.5 MiB peak per writer (plus the short-lived `ledgers` insert at
//! commit), independent of partition row count. Comfortable headroom
//! even at K=16 parallel partitions on a laptop.

use clickhouse::{Client, insert::Insert};

use super::rows::*;
use super::stage::StagedLedger;
use crate::SchemaError;

/// The set of tables a targeted (`--only`) re-parse persists.
///
/// Only tables that are **additive** may be targeted — a new derived table
/// whose rows are deterministic from the XDR, carry no Tier-1 MIN-semantics
/// column, and can be rolled back with `DROP TABLE`. The list is closed on
/// purpose: adding a name here is a statement that the table meets those
/// conditions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetedTables(Vec<&'static str>);

impl TargetedTables {
    pub const TARGETABLE: &'static [&'static str] = &[
        "lp_operation_amounts",
        "asset_transfers",
        "transaction_memos",
        "soroban_event_ops",
    ];
    /// Parse a comma-separated list; rejects unknown or duplicate names.
    pub fn parse(spec: &str) -> Result<Self, String> {
        let mut out: Vec<&'static str> = Vec::new();
        for raw in spec.split(',') {
            let name = raw.trim();
            if name.is_empty() {
                continue;
            }
            let Some(known) = Self::TARGETABLE.iter().copied().find(|t| *t == name) else {
                return Err(format!(
                    "`{name}` is not a targetable table (targetable: {})",
                    Self::TARGETABLE.join(", ")
                ));
            };
            if out.contains(&known) {
                return Err(format!("`{name}` listed twice"));
            }
            out.push(known);
        }
        if out.is_empty() {
            return Err("no table named".into());
        }
        Ok(Self(out))
    }

    pub fn iter(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.0.iter().copied()
    }
}

/// Lifecycle handle for a single 0204-schema partition write.
///
/// Construct with [`PartitionWriter::open`]; stream ledgers through
/// [`PartitionWriter::write_ledger`]; finalise with
/// [`PartitionWriter::commit`] or [`PartitionWriter::abort`] (on any
/// error the caller is responsible for calling `abort()` before
/// dropping the writer — drop alone aborts the in-flight HTTP requests
/// but does not yield diagnostic logs).
pub struct PartitionWriter {
    client: Client,
    inserts: TableInserts,
    /// Ledger rows buffered as the partition's commit marker. Held in
    /// RAM (one `LedgerRow` per call to `write_ledger`) and flushed in
    /// `commit()` after every other insert has ack'd.
    ledger_rows: Vec<LedgerRow>,
}

#[derive(Default)]
struct TableInserts {
    accounts: Option<Insert<AccountRow>>,
    account_entry_state: Option<Insert<AccountEntryStateRow>>,
    wasm: Option<Insert<WasmInterfaceMetadataRow>>,
    contracts: Option<Insert<SorobanContractRow>>,
    metadata: Option<Insert<SorobanContractMetadataRow>>,
    transactions: Option<Insert<TransactionRow>>,
    hash_index: Option<Insert<TransactionHashIndexRow>>,
    participants: Option<Insert<TransactionParticipantRow>>,
    op_assets: Option<Insert<OperationAssetAppearanceRow>>,
    op_pools: Option<Insert<OperationPoolRow>>,
    lp_amounts: Option<Insert<LpOperationAmountRow>>,
    pools: Option<Insert<LiquidityPoolRow>>,
    pool_instance_state: Option<Insert<PoolInstanceStateRow>>,
    pool_state_changes: Option<Insert<PoolStateChangeRow>>,
    snapshots: Option<Insert<LiquidityPoolSnapshotRow>>,
    lp_positions: Option<Insert<LpPositionRow>>,
    operations: Option<Insert<OperationAppearanceRow>>,
    events: Option<Insert<SorobanEventRow>>,
    invocations: Option<Insert<SorobanInvocationAppearanceRow>>,
    assets: Option<Insert<AssetRow>>,
    asset_sac: Option<Insert<AssetSacRow>>,
    nfts: Option<Insert<NftRow>>,
    nft_ownership: Option<Insert<NftOwnershipRow>>,
    /// Task 0217 / 0220 — quarantine inserts. Lazy-opened only when the
    /// `stage::prepare` routing actually produces pending rows for the
    /// partition (any contract still classified `Other` / NULL at the
    /// time of staging). Empty `Other`-free partitions never open the
    /// HTTP request, keeping the part economy unchanged from PR #180.
    nfts_pending: Option<Insert<NftPendingRow>>,
    nft_ownership_pending: Option<Insert<NftOwnershipPendingRow>>,
    /// Unified per-holder balances — ALL asset types (task 0331 Option A). The
    /// legacy `account_balances_current` insert was removed (single-write).
    unified_balances: Option<Insert<BalanceRow>>,
    /// Task 0540 / 0541 — the value-flow tables.
    asset_transfers: Option<Insert<AssetTransferRow>>,
    transaction_memos: Option<Insert<TransactionMemoRow>>,
    event_ops: Option<Insert<SorobanEventOpRow>>,
}

impl PartitionWriter {
    /// Construct a writer attached to `client`. No HTTP requests are
    /// issued until the first row is written.
    ///
    /// `client` is held by value (`Client` is `Clone` and the
    /// underlying hyper connection pool is `Arc`'d). The writer
    /// applies its own bulk-ingest CH settings on every per-table
    /// insert it opens — see [`apply_bulk_ingest_settings`].
    pub fn open(client: Client) -> Self {
        Self {
            client,
            inserts: TableInserts::default(),
            ledger_rows: Vec::new(),
        }
    }

    /// The CH client this writer holds. Lets the backfill `Run` path issue
    /// side queries (e.g. the ADR 0051 `asset_sac` SAC→classic prefetch)
    /// on the same connection without threading a second handle through.
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Stream one ledger's staged rows into the open inserts.
    ///
    /// Cheap — rows go into each insert's RowBinary buffer and the
    /// crate chunk-sends them over HTTP transparently when the buffer
    /// fills. `ledgers` rows are **not** sent during this call;
    /// they're buffered as the partition's commit marker.
    /// Stream ONLY the named tables' rows for this ledger — the targeted write
    /// a historical re-parse for new derived tables runs (task 0279 set the
    /// pattern with `lp_operation_amounts`; task 0540 generalised it to a list
    /// so `asset_transfers`, `transaction_memos` and `soroban_event_ops` ride
    /// one pass).
    ///
    /// Two things this deliberately does NOT do, both load-bearing:
    ///
    /// - **No other table** — a full re-emission would rewrite the 12 Tier-1
    ///   columns that cannot survive parallel `ReplacingMergeTree` collapse
    ///   (`docs/backfills.md` §3), turning an additive backfill into one that
    ///   owes a `repair-tier1` pass.
    /// - **No `ledgers` commit marker** — the marker means "fully ingested",
    ///   which a targeted pass has not done. The cost is that resume cannot
    ///   read progress from the DB: a crashed targeted run resumes by
    ///   narrowing `--start`, and re-running a range is a no-op **for one
    ///   decoder version** (identical rows collapse in the RMT). After a
    ///   decoder change the old and new rows share a key and differ in
    ///   content, and no column says which is which — run the 0503 tie
    ///   query (`docs/backfills.md`, "keys with more than one distinct
    ///   content") before trusting a re-run.
    ///
    /// Table names are validated by [`TargetedTables::parse`] before a run
    /// starts, so an unknown name never reaches this method.
    pub async fn write_only(
        &mut self,
        staged: &StagedLedger,
        only: &TargetedTables,
    ) -> Result<(), SchemaError> {
        for table in only.iter() {
            match table {
                "lp_operation_amounts" => {
                    write_rows(
                        &self.client,
                        &mut self.inserts.lp_amounts,
                        "lp_operation_amounts",
                        &staged.lp_amount_rows,
                    )
                    .await?
                }
                "asset_transfers" => {
                    write_rows(
                        &self.client,
                        &mut self.inserts.asset_transfers,
                        "asset_transfers",
                        &staged.asset_transfer_rows,
                    )
                    .await?
                }
                "transaction_memos" => {
                    write_rows(
                        &self.client,
                        &mut self.inserts.transaction_memos,
                        "transaction_memos",
                        &staged.transaction_memo_rows,
                    )
                    .await?
                }
                "soroban_event_ops" => {
                    write_rows(
                        &self.client,
                        &mut self.inserts.event_ops,
                        "soroban_event_ops",
                        &staged.event_op_rows,
                    )
                    .await?
                }
                other => {
                    return Err(SchemaError::Staging(format!(
                        "targeted write: table {other} is not targetable"
                    )));
                }
            }
        }
        Ok(())
    }

    pub async fn write_ledger(&mut self, mut staged: StagedLedger) -> Result<(), SchemaError> {
        // Hold the ledger row(s) back as commit marker.
        self.ledger_rows.append(&mut staged.ledger_rows);

        // EXHAUSTIVE destructure — the other half of `commit()`'s invariant
        // (three-lens review, 2026-09-01): a future `StagedLedger` row-vec
        // that never reaches a `write_rows` call refuses to compile here,
        // instead of being staged and dropped silently one hop upstream of
        // the bug class the 0374 e2e caught in `commit()`.
        let StagedLedger {
            ledger_sequence: _,
            ledger_rows: _,
            account_rows,
            account_entry_state_rows,
            wasm_rows,
            contract_rows,
            metadata_rows,
            transaction_rows,
            hash_index_rows,
            participant_rows,
            pool_rows,
            pool_instance_state_rows,
            pool_state_change_rows,
            snapshot_rows,
            lp_position_rows,
            op_rows,
            op_asset_rows,
            op_pool_rows,
            lp_amount_rows,
            event_rows,
            invocation_rows,
            asset_rows,
            asset_sac_rows,
            nft_rows,
            nft_ownership_rows,
            nft_pending_rows,
            nft_ownership_pending_rows,
            unified_balance_rows,
            asset_transfer_rows,
            transaction_memo_rows,
            event_op_rows,
        } = staged;

        write_rows(
            &self.client,
            &mut self.inserts.accounts,
            "accounts",
            &account_rows,
        )
        .await?;
        write_rows(
            &self.client,
            &mut self.inserts.account_entry_state,
            "account_entry_state",
            &account_entry_state_rows,
        )
        .await?;
        write_rows(
            &self.client,
            &mut self.inserts.wasm,
            "wasm_interface_metadata",
            &wasm_rows,
        )
        .await?;
        write_rows(
            &self.client,
            &mut self.inserts.contracts,
            "soroban_contracts",
            &contract_rows,
        )
        .await?;
        write_rows(
            &self.client,
            &mut self.inserts.metadata,
            "soroban_contract_metadata",
            &metadata_rows,
        )
        .await?;
        write_rows(
            &self.client,
            &mut self.inserts.transactions,
            "transactions",
            &transaction_rows,
        )
        .await?;
        write_rows(
            &self.client,
            &mut self.inserts.hash_index,
            "transaction_hash_index",
            &hash_index_rows,
        )
        .await?;
        write_rows(
            &self.client,
            &mut self.inserts.participants,
            "transaction_participants",
            &participant_rows,
        )
        .await?;
        write_rows(
            &self.client,
            &mut self.inserts.op_assets,
            "operation_asset_appearances",
            &op_asset_rows,
        )
        .await?;
        write_rows(
            &self.client,
            &mut self.inserts.op_pools,
            "operation_pools",
            &op_pool_rows,
        )
        .await?;
        write_rows(
            &self.client,
            &mut self.inserts.lp_amounts,
            "lp_operation_amounts",
            &lp_amount_rows,
        )
        .await?;
        write_rows(
            &self.client,
            &mut self.inserts.pools,
            "liquidity_pools",
            &pool_rows,
        )
        .await?;
        write_rows(
            &self.client,
            &mut self.inserts.pool_instance_state,
            "pool_instance_state",
            &pool_instance_state_rows,
        )
        .await?;
        write_rows(
            &self.client,
            &mut self.inserts.pool_state_changes,
            "pool_state_changes",
            &pool_state_change_rows,
        )
        .await?;
        write_rows(
            &self.client,
            &mut self.inserts.snapshots,
            "liquidity_pool_snapshots",
            &snapshot_rows,
        )
        .await?;
        write_rows(
            &self.client,
            &mut self.inserts.lp_positions,
            "lp_positions",
            &lp_position_rows,
        )
        .await?;
        write_rows(
            &self.client,
            &mut self.inserts.operations,
            "operations_appearances",
            &op_rows,
        )
        .await?;
        write_rows(
            &self.client,
            &mut self.inserts.events,
            "soroban_events",
            &event_rows,
        )
        .await?;
        write_rows(
            &self.client,
            &mut self.inserts.invocations,
            "soroban_invocations_appearances",
            &invocation_rows,
        )
        .await?;

        write_rows(
            &self.client,
            &mut self.inserts.assets,
            "assets",
            &asset_rows,
        )
        .await?;
        write_rows(
            &self.client,
            &mut self.inserts.asset_sac,
            "asset_sac",
            &asset_sac_rows,
        )
        .await?;
        write_rows(&self.client, &mut self.inserts.nfts, "nfts", &nft_rows).await?;

        write_rows(
            &self.client,
            &mut self.inserts.nft_ownership,
            "nft_ownership",
            &nft_ownership_rows,
        )
        .await?;
        // Task 0217 / 0220 — quarantine inserts. Slot stays `None` (and
        // the HTTP request never opens) on partitions where every
        // NFT-candidate contract has a definitive `Nft` verdict.
        write_rows(
            &self.client,
            &mut self.inserts.nfts_pending,
            "nfts_pending",
            &nft_pending_rows,
        )
        .await?;
        write_rows(
            &self.client,
            &mut self.inserts.nft_ownership_pending,
            "nft_ownership_pending",
            &nft_ownership_pending_rows,
        )
        .await?;
        write_rows(
            &self.client,
            &mut self.inserts.unified_balances,
            "balances",
            &unified_balance_rows,
        )
        .await?;
        write_rows(
            &self.client,
            &mut self.inserts.asset_transfers,
            "asset_transfers",
            &asset_transfer_rows,
        )
        .await?;
        write_rows(
            &self.client,
            &mut self.inserts.transaction_memos,
            "transaction_memos",
            &transaction_memo_rows,
        )
        .await?;
        write_rows(
            &self.client,
            &mut self.inserts.event_ops,
            "soroban_event_ops",
            &event_op_rows,
        )
        .await?;

        Ok(())
    }

    /// End every non-`ledgers` insert in PG-FK-friendly order, then
    /// open + write + end the `ledgers` insert as the commit marker.
    ///
    /// Order rationale: PG enforces FKs, CH does not, but reads (and
    /// any future re-introduced enforcement) are easier when the
    /// referenced rows land before the referencing ones. The `ledgers`
    /// row is the marker the resume path scans for, so it must be the
    /// **last** thing to acknowledge — a mid-commit failure between
    /// step 1 and step 2 below leaves no `ledgers` rows for the
    /// partition and the resume path re-does the whole range.
    pub async fn commit(self) -> Result<(), SchemaError> {
        // Step 1: drain every non-ledger insert. Order roughly mirrors
        // PG's write order (accounts → wasm → contracts → tx → hash
        // index → participants → pools/snapshots/positions → ops →
        // events → invocations → assets → nfts/ownership → balances).
        //
        // EXHAUSTIVE destructure, deliberately no `..`: an insert that is
        // written but never ended buffers its rows and drops them SILENTLY
        // on drop — the ledgers marker still lands, so the loss is
        // invisible. That exact bug shipped twice (the instance-state side
        // table and
        // pool_state_changes were streamed by `write_ledger` but missing
        // from this list; caught by the task 0374 full-pipeline e2e, never
        // by unit tests, which stop at staging). With the destructure the
        // compiler refuses a new `TableInserts` field until someone decides
        // where it drains.
        let TableInserts {
            accounts,
            account_entry_state,
            wasm,
            contracts,
            metadata,
            transactions,
            hash_index,
            participants,
            op_assets,
            op_pools,
            lp_amounts,
            pools,
            pool_instance_state,
            pool_state_changes,
            snapshots,
            lp_positions,
            operations,
            events,
            invocations,
            assets,
            asset_sac,
            nfts,
            nft_ownership,
            nfts_pending,
            nft_ownership_pending,
            unified_balances,
            asset_transfers,
            transaction_memos,
            event_ops,
        } = self.inserts;
        end(accounts).await?;
        end(account_entry_state).await?;
        end(wasm).await?;
        end(contracts).await?;
        end(metadata).await?;
        end(transactions).await?;
        end(hash_index).await?;
        end(participants).await?;
        end(op_assets).await?;
        end(op_pools).await?;
        end(lp_amounts).await?;
        end(pools).await?;
        end(pool_instance_state).await?;
        end(pool_state_changes).await?;
        end(snapshots).await?;
        end(lp_positions).await?;
        end(operations).await?;
        end(events).await?;
        end(invocations).await?;
        end(assets).await?;
        end(asset_sac).await?;
        end(nfts).await?;
        end(nft_ownership).await?;
        // Task 0217 / 0220 — drain quarantine inserts in the same
        // pre-`ledgers` step. They share the commit-marker guarantee:
        // a partial commit that fails between any of these and the
        // final `ledgers` write produces no `ledgers` row for the
        // partition, so the resume path re-does it cleanly. RMT
        // dedupes the orphan rows on the next merge.
        end(nfts_pending).await?;
        end(nft_ownership_pending).await?;
        end(unified_balances).await?;
        end(asset_transfers).await?;
        end(transaction_memos).await?;
        end(event_ops).await?;

        // Step 2: commit marker. Open `ledgers` insert, write every
        // buffered row, end the request.
        if !self.ledger_rows.is_empty() {
            let mut insert = open_insert::<LedgerRow>(&self.client, "ledgers").await?;
            for row in &self.ledger_rows {
                insert.write(row).await?;
            }
            insert.end().await?;
        }
        Ok(())
    }

    /// Abandon the partition without ending any insert. Drops every
    /// `Insert` handle, which the `clickhouse` crate documents as
    /// aborting the in-flight HTTP request. Use this on any per-ledger
    /// write error to avoid leaving open connections in flight.
    ///
    /// Note: `ledgers` rows are not written, so resume will redo this
    /// partition cleanly.
    pub async fn abort(self) {
        // Drop is enough; the crate aborts on drop without `end()`.
        // We model this as an `async fn` for symmetry with
        // `commit()` — callers can `.await?` either path uniformly.
        drop(self.inserts);
        drop(self.ledger_rows);
    }
}

/// Lazy-open an insert and stream rows into its buffer.
///
/// `T: Row + RowOwned + Serialize` — the row structs in [`super::rows`]
/// are owned types (no borrows), so `clickhouse`'s blanket impls give
/// us `T::Value<'a> = T` and `T: RowWrite`. That lets `insert.write(&row)`
/// take `&T` directly without lifetime gymnastics in this helper.
async fn write_rows<T>(
    client: &Client,
    slot: &mut Option<Insert<T>>,
    table: &'static str,
    rows: &[T],
) -> Result<(), SchemaError>
where
    T: clickhouse::Row + clickhouse::RowOwned + serde::Serialize,
{
    if rows.is_empty() {
        return Ok(());
    }
    if slot.is_none() {
        *slot = Some(open_insert::<T>(client, table).await?);
    }
    let insert = slot.as_mut().expect("just-opened insert");
    for row in rows {
        insert.write(row).await?;
    }
    Ok(())
}

/// Build a new `Insert<T>` against `table` with the bulk-ingest CH
/// settings applied uniformly.
async fn open_insert<T>(client: &Client, table: &'static str) -> Result<Insert<T>, SchemaError>
where
    T: clickhouse::Row + clickhouse::RowOwned,
{
    let insert = client.insert::<T>(table).await?;
    Ok(apply_bulk_ingest_settings(insert))
}

/// Apply CH server-side settings that move the merger from "drowning
/// in small parts" territory to "comfortable" for the backfill path:
///
/// * `async_insert = 0` — we batch client-side; server-side
///   async-buffer would add latency variance without gain at this
///   batch size.
/// * `max_insert_block_size = 1_048_576` — server-side block size cap
///   on INSERT processing. Pinned to lock against future CH default
///   changes.
/// * `min_insert_block_size_rows = 1_000_000` — coalesce small chunked
///   pieces into 1 M-row blocks before the part-create path.
/// * `min_insert_block_size_bytes = 268_435_456` — same coalescing
///   knob, byte side (256 MiB).
/// * `insert_deduplicate = 0` — we rely on ReplacingMergeTree dedup
///   keyed by `ORDER BY`, not on the per-block dedup hash (which
///   would consume RAM for our row volumes).
/// * `http_receive_timeout = 7200` / `http_send_timeout = 7200` —
///   long-running partition-aligned inserts can have multi-minute
///   gaps between chunked HTTP body writes on **sparse** tables
///   (e.g. `nfts` with ~15 rows/ledger fills the crate's ~256 KiB
///   buffer only every ~5 minutes; `wasm_interface_metadata` and
///   `lp_positions` can sit empty across many ledgers). CH's default
///   `http_receive_timeout = 30s` then closes the socket
///   server-side between sparse chunks, surfacing on the client as
///   `Network("channel closed")` after about 10 minutes of run on a
///   real mainnet partition — observed at ~9.6 k / 10 k ledgers on
///   the 62016000 smoke. 2 hours covers a 64 k-ledger partition
///   (~80 min wall-clock) with headroom for future K=4 parallel
///   contention; the timeout is per-stalled-chunk, not per-request,
///   so steady-state never approaches it. Note: per-query
///   `with_setting` does NOT propagate to the Poco socket read on
///   CH 26.3 — only the profile-level XML in `users.d/timeouts.xml`
///   takes effect on the wire. The settings here are belt-and-
///   suspenders for future CH versions that may honour them.
///
/// Compression is not a lever here, and the setting that looks like one
/// is not. `enable_http_compression` is a CH server setting governing
/// **response** bodies; it does not touch the INSERT request body. The
/// `clickhouse` crate compresses responses only — `default = ["lz4"]`,
/// and `src/insert.rs` carries no compression path at all (unlike
/// `src/insert_formatted.rs`). So a typed `client.insert::<T>()` body
/// always goes out uncompressed regardless of settings. On the loopback
/// backfill this costs nothing; from a Lambda across the public internet
/// it is paid egress. Documented in `crates/db-clickhouse/README.md`.
fn apply_bulk_ingest_settings<T>(insert: Insert<T>) -> Insert<T> {
    insert
        .with_setting("async_insert", "0")
        .with_setting("max_insert_block_size", "1048576")
        .with_setting("min_insert_block_size_rows", "1000000")
        .with_setting("min_insert_block_size_bytes", "268435456")
        .with_setting("insert_deduplicate", "0")
        .with_setting("http_receive_timeout", "7200")
        .with_setting("http_send_timeout", "7200")
}

async fn end<T>(slot: Option<Insert<T>>) -> Result<(), SchemaError> {
    if let Some(insert) = slot {
        insert.end().await?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "writer_tests.rs"]
mod tests;
