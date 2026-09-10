//! backfill-runner — production-grade Stellar pubnet backfill to ClickHouse.
//!
//! Source: `aws-public-blockchain/v1.1/stellar/ledgers/pubnet/` (unsigned).
//! Sink:   ClickHouse (ADR 0044), via the `db_clickhouse::persist`
//!         partition-writer lifecycle.

mod balance_seed;
mod bootstrap;
mod ch_staging;
mod contract_type_rebuild;
mod dashboard;
mod error;
mod ingest;
mod nft_reclassify;
mod partition;
mod pool_legs_fill;
mod repair_tier1;
mod rpc_snapshot;
mod run;
mod sink;
mod snapshot;
mod status;
mod sync;
mod util;

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use db_clickhouse::persist::TargetedTables;

/// Default local scratch dir. CLI `--temp-dir` or `BACKFILL_TEMP_DIR`
/// overrides. Single source of truth — `run` and `status` both receive
/// it via their `execute` args, no duplicated constant.
const DEFAULT_TEMP_DIR: &str = ".temp/backfill-runner";
const DEFAULT_ARTIFACTS_DIR: &str = ".artifacts/snapshot-seed";

#[derive(Parser)]
#[command(name = "backfill-runner", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// ClickHouse HTTP endpoint (e.g. `http://localhost:8123`).
    /// Overrides `CLICKHOUSE_URL` for the duration of the run. Other
    /// ClickHouse env vars (`CLICKHOUSE_USER`, `CLICKHOUSE_PASSWORD`,
    /// `CLICKHOUSE_DATABASE`) are picked up by
    /// `db_clickhouse::Config::from_env()` as usual.
    #[arg(long, env = "CLICKHOUSE_URL")]
    clickhouse_url: Option<String>,

    /// Client certificate (PEM) for mTLS to the Caddy-fronted ClickHouse
    /// (`https://<CH_DOMAIN>`). When all of `--ch-cert` / `--ch-key` /
    /// `--ch-ca` are set, the `--target clickhouse` sink connects over mTLS
    /// (the cert CN maps to a CH user via Caddy's `CLICKHOUSE_CN_USER_MAP`)
    /// instead of the plain HTTP client; `--clickhouse-url` / `CLICKHOUSE_URL`
    /// must then be the https Caddy endpoint. All three absent → plain client.
    #[arg(long, env = "CLICKHOUSE_CERT")]
    ch_cert: Option<PathBuf>,

    /// Client private key (PEM) — pairs with `--ch-cert`.
    #[arg(long, env = "CLICKHOUSE_KEY")]
    ch_key: Option<PathBuf>,

    /// CA cert (PEM) that signed the client cert — pairs with `--ch-cert`.
    #[arg(long, env = "CLICKHOUSE_CA")]
    ch_ca: Option<PathBuf>,

    /// Soroban RPC endpoint (e.g.
    /// `https://soroban-rpc.mainnet.stellar.gateway.fm`). Used by the
    /// CH bootstrap step (task 0214, audit §E06) to fetch live
    /// `AccountEntry` state for accounts referenced in the window
    /// but never updated inside it. Optional — when unset, the
    /// bootstrap step is skipped and accounts persist as the
    /// participants-driven skeleton rows.
    #[arg(long, env = "SOROBAN_RPC_URL")]
    soroban_rpc_url: Option<String>,

    /// Local scratch directory for `aws s3 sync` output. Each partition
    /// lands under `<temp-dir>/<HEX>--<start>-<end>/` and (by default)
    /// is deleted after it indexes successfully.
    #[arg(long, env = "BACKFILL_TEMP_DIR", default_value = DEFAULT_TEMP_DIR)]
    temp_dir: PathBuf,

    /// Keep each partition's local folder on disk after it finishes
    /// indexing. Default: delete (bounds disk at ~2 × partition_size).
    ///
    /// Intended for iteration / debugging — most useful when re-running
    /// the same range repeatedly (e.g. `--target clickhouse` stub work):
    /// the next `aws s3 sync` against a fully-populated folder is a cheap
    /// LIST instead of an 11.6 GB / 60 s re-download per partition.
    /// **Do not pass this for a real backfill** — disk grows linearly
    /// with the number of indexed partitions.
    #[arg(long)]
    keep_partitions: bool,

    /// Persist ONLY the named tables (comma-separated) — the targeted write a
    /// historical re-parse for new derived tables needs. Task 0279 introduced
    /// it as `--lp-amounts-only`; task 0540 generalised it so its three tables
    /// ride one pass: `--only asset_transfers,transaction_memos,soroban_event_ops`.
    /// `--only lp_operation_amounts` is the old behaviour.
    ///
    /// Without it, `run --reindex` re-emits EVERY table, which rewrites the 12
    /// Tier-1 columns that cannot survive parallel `ReplacingMergeTree`
    /// collapse and so owes a `repair-tier1` pass afterwards
    /// (`docs/backfills.md` §3). With it the run is additive: named tables
    /// only, no MIN-semantics column, indexer untouched, rollback is
    /// `DROP TABLE`. Only tables that meet those conditions are accepted
    /// (`TargetedTables::TARGETABLE`).
    ///
    /// Implies `--reindex` (the range is already ingested, so the resume
    /// filter would otherwise skip every ledger). No `ledgers` commit marker
    /// is written, so a crashed run resumes by narrowing `--start` — re-running
    /// a range is a no-op, the rows are deterministic and the RMT collapses
    /// them.
    #[arg(long, value_parser = TargetedTables::parse)]
    only: Option<TargetedTables>,

    /// Enable per-ledger and per-partition progress logs. Without this
    /// flag only warnings are shown during the run; the final summary
    /// (and the `status` table) prints either way.
    #[arg(long, short)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Run the backfill for a sequence range.
    Run {
        /// First ledger sequence (inclusive).
        #[arg(long)]
        start: u32,

        /// Last ledger sequence (inclusive).
        #[arg(long)]
        end: u32,

        /// Re-index ledgers already present in the `ledgers` table. The resume
        /// check normally skips ledgers already ingested (Run is a gap-filler),
        /// so re-parsing an already-ingested range to populate a NEW derived
        /// table over history (e.g. task 0359 `operation_asset_appearances`)
        /// would be a no-op. Pass this to bypass the resume skip and re-parse
        /// the full range. All writes are ReplacingMergeTree-idempotent.
        #[arg(long)]
        reindex: bool,
    },

    /// Report ingested / missing ledgers for a range.
    Status {
        /// First ledger sequence (inclusive).
        #[arg(long)]
        start: u32,

        /// Last ledger sequence (inclusive).
        #[arg(long)]
        end: u32,
    },

    /// Run only the account-state bootstrap RPC pass on an existing CH
    /// dataset. Top-ups `sequence_number = 0` skeletons via Soroban RPC
    /// `getLedgerEntries`. Idempotent — skips accounts where the
    /// in-window parser path already filled real state. Useful when an
    /// earlier `Run` was invoked without `--soroban-rpc-url` and left
    /// the dataset with elevated skeleton counts.
    ///
    /// No-op when `--soroban-rpc-url` is unset.
    Bootstrap {
        /// First ledger sequence (inclusive). Used by the discovery
        /// query's `transaction_participants` JOIN.
        #[arg(long)]
        start: u32,

        /// Last ledger sequence (inclusive). Snapshot stamp is
        /// `max(end + 1, current SELECT max(last_seen_ledger) FROM
        /// accounts + 1)` to win the RMT(last_seen_ledger) race even
        /// after partial-commit crash recovery (where parser writes
        /// can land beyond the last committed tx ledger).
        #[arg(long)]
        end: u32,
    },

    /// Tier-1 post-merge column rebuild for the Hetzner CH
    /// (task 0228 Phase 5). Reconstructs 6 of the 12 Tier-1 columns
    /// across 5 state tables (`accounts.first_seen_ledger`,
    /// `lp_positions.first_deposit_ledger`,
    /// `nfts.minted_at_ledger`, `nfts_pending.minted_at_ledger`,
    /// `soroban_contracts.deployer_id` + `deployed_at_ledger`).
    /// These silently corrupt under cross-machine
    /// `ReplacingMergeTree` collapse. The remaining 6 columns
    /// (NFT metadata: `collection_name`, `name`, `media_url` × 2
    /// tables) are filled by Stage 2 enrichment (task 0231).
    /// Per-table staging + EXCHANGE TABLES atomic swap.
    RepairTier1 {
        /// Build staging tables and log their row counts, then drop
        /// them — do not EXCHANGE. Use on laptop 1's local CH as a
        /// sandbox before running for real on Hetzner.
        #[arg(long)]
        dry_run: bool,
    },

    /// One-shot rebuild of `soroban_contracts.contract_type` from
    /// `wasm_interface_metadata` + `assets` type-3 backfill (task 0283).
    /// Classifies every WASM in Rust (parity with the parser), rebuilds
    /// `soroban_contracts` into staging and `EXCHANGE TABLES`-swaps it, then
    /// inserts the missing Soroban-fungible `assets` rows. Must run BEFORE
    /// `nft-reclassify` (which promotes `contract_type = 2`), with the indexer
    /// STOPPED (whole-table swap). Idempotent; `--dry-run` reports verdict
    /// transitions + would-be asset inserts without writing. CH-only.
    /// Task 0374 — fill `liquidity_pools.legs` for classic pools that still
    /// carry the empty default, from the pair columns.
    ///
    /// Run with the indexer STOPPED (atomic table swap), and BEFORE the pair
    /// columns are dropped — it reads the very columns that migration removes.
    /// This is what makes the deploy gate
    /// (`countIf(length(legs) = 0)` = 0) reachable: `legs` is filled at write
    /// time, so a pool that stopped trading never gets it on its own.
    PoolLegsFill {
        /// Report what would be filled; leave the live table untouched.
        #[arg(long)]
        dry_run: bool,
    },
    ContractTypeRebuild {
        #[arg(long)]
        dry_run: bool,
    },

    /// Step 3d — THE seed (ADR 0055): build every correction the comparison
    /// proved necessary. Reads our balances and dimension ids straight from
    /// ClickHouse — no manual exports. Default is a DRY-RUN that reads,
    /// decodes, folds and writes artifacts (manifest.json, summary.txt,
    /// ghosts.tsv) without inserting anything; `--execute` performs the
    /// inserts. Deployment order is load-bearing: deploy the lifecycle writer
    /// FIRST, then seed from a checkpoint taken after that deploy (see the
    /// module docs).
    SnapshotSeed {
        /// Parent directory for provenance artifacts. Each run writes into
        /// `<artifacts>/<checkpoint_ledger>/` (manifest, summary, ghost list,
        /// dumps), so a run never overwrites the record of an earlier one.
        #[arg(long, env = "BACKFILL_ARTIFACTS_DIR", default_value = DEFAULT_ARTIFACTS_DIR)]
        artifacts: PathBuf,
        /// Actually insert. Without this flag the run is read-only.
        #[arg(long)]
        execute: bool,
    },

    /// Task 0331 step 7 — one-shot RPC-snapshot seed of per-holder balances into
    /// the unified `balances` table: bespoke type-3 Soroban tokens AND contract-held
    /// classic/native (types 0/1, held via each asset's SAC — re-keyed onto the
    /// wrapped asset via `asset_sac`, ADR 0051). Enumerates holders from the
    /// `soroban_events` stream, reads their current `Balance(Address)` ledger
    /// entries from Soroban RPC (`getLedgerEntries`), and upserts `balances`
    /// (ReplacingMergeTree). Reads CURRENT chain state, so it is correct regardless
    /// of indexer lag; live ingest supersedes it on catch-up. Requires
    /// `--soroban-rpc-url`. Idempotent. `--dry-run` reports without writing.
    /// CH-only — a non-ClickHouse target errors (`Incomplete`), it does NOT no-op.
    BalanceSeed {
        #[arg(long)]
        dry_run: bool,
    },

    /// Post-merge NFT reclassification on the Hetzner CH (task 0228
    /// Phase 5; combines task 0118 Phase 3 cleanup with task 0217
    /// quarantine promotion):
    ///
    /// - Promote `nfts_pending` rows → `nfts` for contracts now
    ///   classified `Nft`.
    /// - Drop pending rows for contracts now `Fungible` or `Token`.
    /// - Drop legacy false positives from hot `nfts` / `nft_ownership`.
    ///
    /// Uses `ALTER TABLE … DELETE` with `mutations_sync = 1` followed
    /// by `OPTIMIZE FINAL` to collapse tombstones. CH-only.
    NftReclassify {
        #[arg(long)]
        dry_run: bool,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Shared MultiProgress so the tracing writer and the run-level progress
    // bar coordinate: every tracing write suspends the bar, renders the log
    // line, then redraws the bar on the last line. Without this the bar
    // "streams" — each redraw appears on a new line below the previous log,
    // leaving a trail instead of one sticky bar at the bottom.
    let mp = indicatif::MultiProgress::new();
    // Type annotation is load-bearing — `IndicatifWriter::new` returns
    // `IndicatifWriter<W>` where `W` defaults to `Stderr` only via the
    // `Default` bound on a separate constructor; here Rust can't infer
    // it from `with_writer` downstream. Drop the annotation and
    // tracing-subscriber's `init()` fails with E0283.
    let writer: tracing_indicatif::writer::IndicatifWriter<tracing_indicatif::writer::Stderr> =
        tracing_indicatif::writer::IndicatifWriter::new(mp.clone());

    let filter = if cli.verbose { "info" } else { "warn" };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .init();

    // Errors currently panic (see task 0145, debug-first decision). The
    // subcommand entrypoints still return `Result` for pool / IO wiring;
    // `.expect` converts any residual Err into an immediate panic with a
    // clear message and no graceful-exit path.
    let sink = build_sink(
        cli.clickhouse_url.as_deref(),
        cli.ch_cert.as_deref(),
        cli.ch_key.as_deref(),
        cli.ch_ca.as_deref(),
    )
    .with_only(cli.only.clone());

    match cli.command {
        Command::Run {
            start,
            end,
            reindex,
        } => run::execute(
            &sink,
            &cli.temp_dir,
            start,
            end,
            cli.keep_partitions,
            cli.soroban_rpc_url.as_deref(),
            reindex,
            &mp,
        )
        .await
        .expect("backfill run failed"),
        Command::Status { start, end } => status::execute(&sink, start, end)
            .await
            .expect("status failed"),
        Command::Bootstrap { start, end } => {
            let rpc_url = cli.soroban_rpc_url.as_deref().unwrap_or_else(|| {
                panic!("bootstrap subcommand requires --soroban-rpc-url (or SOROBAN_RPC_URL env)")
            });
            let stats = bootstrap::bootstrap_account_state(&sink, Some(rpc_url), start, end)
                .await
                .expect("bootstrap failed");
            // Print one-line summary on stdout so operators piping into
            // shell scripts see the result without grep'ing tracing
            // output.
            println!(
                "bootstrap completed: discovered={} fetched={} staged={} rpc_batches={} rpc_errors={}",
                stats.discovered,
                stats.fetched,
                stats.staged_accounts,
                stats.rpc_batches,
                stats.rpc_errors,
            );
        }
        Command::RepairTier1 { dry_run } => {
            let stats = repair_tier1::execute(&sink, dry_run)
                .await
                .expect("repair_tier1 failed");
            println!(
                "repair_tier1 completed (dry_run={}): accounts={} lp_positions={} nfts={} nfts_pending={} soroban_contracts={}",
                stats.dry_run,
                stats.accounts_rows,
                stats.lp_positions_rows,
                stats.nfts_rows,
                stats.nfts_pending_rows,
                stats.soroban_contracts_rows,
            );
        }
        Command::PoolLegsFill { dry_run } => {
            let stats = pool_legs_fill::execute(&sink, dry_run)
                .await
                .expect("pool_legs_fill failed — idempotent, safe to re-run");
            println!(
                "pool_legs_fill completed (dry_run={}): unfilled_before={} filled={} unresolvable={}",
                stats.dry_run, stats.unfilled_before, stats.filled, stats.unresolvable,
            );
        }
        Command::ContractTypeRebuild { dry_run } => {
            let stats = contract_type_rebuild::execute(&sink, dry_run).await.expect(
                "contract_type_rebuild failed — if it failed AFTER the table \
                     swap (e.g. during the assets backfill), simply re-run: the \
                     pass is idempotent (re-flip is a no-op, assets insert is \
                     NOT EXISTS-guarded)",
            );
            println!(
                "contract_type_rebuild completed (dry_run={}): flipped_nft={} flipped_fungible={} assets_inserted={}",
                stats.dry_run, stats.flipped_nft, stats.flipped_fungible, stats.assets_inserted,
            );
        }
        Command::SnapshotSeed { artifacts, execute } => {
            snapshot::seed::seed_command(&sink, &artifacts, execute)
                .await
                .expect("snapshot seed failed");
        }
        Command::BalanceSeed { dry_run } => {
            // CH-only: `execute` hard-fails (`Incomplete`) on a non-ClickHouse
            // target, then enforces `--soroban-rpc-url`, so just pass it through.
            let stats = balance_seed::execute(&sink, cli.soroban_rpc_url.as_deref(), dry_run)
                .await
                .expect("balance_seed failed — idempotent, safe to re-run");
            println!(
                "balance_seed completed (dry_run={}): tokens={} holders_enumerated={} \
                 keys_requested={} entries_returned={} balances_decoded={}",
                stats.dry_run,
                stats.tokens,
                stats.holders_enumerated,
                stats.keys_requested,
                stats.entries_returned,
                stats.balances_decoded,
            );
        }
        Command::NftReclassify { dry_run } => {
            let stats = nft_reclassify::execute(&sink, dry_run)
                .await
                .expect("nft_reclassify failed");
            println!(
                "nft_reclassify completed (dry_run={}): promoted_nfts={} promoted_ownership={} dropped_pending_nfts={} dropped_pending_ownership={} dropped_legacy_nfts={} dropped_legacy_ownership={}",
                stats.dry_run,
                stats.promoted_nfts,
                stats.promoted_ownership,
                stats.dropped_pending_nfts,
                stats.dropped_pending_ownership,
                stats.dropped_legacy_nfts,
                stats.dropped_legacy_ownership,
            );
        }
    }
}

/// Build the ClickHouse `Sink`. Panics loudly at startup on a bad config
/// (mismatched mTLS flags, unreadable PEM) — same posture as the existing
/// pre-flight panics.
///
/// Reads ClickHouse env vars (user, password, database) via
/// `db_clickhouse::Config::from_env`; the `--clickhouse-url` CLI flag already
/// overrides `CLICKHOUSE_URL` for the URL field because clap reads the same
/// env var.
///
/// When `ch_cert` + `ch_key` + `ch_ca` are all supplied (task 0307), the sink
/// connects over mTLS to the Caddy-fronted endpoint: the PEMs are read into an
/// `MtlsBundle` and `client_with_mtls` presents the client cert (whose CN Caddy
/// maps to a CH user via `CLICKHOUSE_CN_USER_MAP`). `cfg.url` must be the https
/// Caddy host; user/password are ignored on that path.
fn build_sink(
    clickhouse_url: Option<&str>,
    ch_cert: Option<&Path>,
    ch_key: Option<&Path>,
    ch_ca: Option<&Path>,
) -> sink::Sink {
    let mut cfg = db_clickhouse::Config::from_env();
    if let Some(url) = clickhouse_url {
        cfg.url = url.to_string();
    }
    match (ch_cert, ch_key, ch_ca) {
        (Some(cert), Some(key), Some(ca)) => {
            let read = |p: &Path| {
                std::fs::read_to_string(p)
                    .unwrap_or_else(|e| panic!("read mTLS PEM {}: {e}", p.display()))
            };
            let bundle = db_clickhouse::mtls::MtlsBundle {
                cert_pem: read(cert),
                key_pem: read(key),
                ca_pem: read(ca),
            };
            // `client_with_mtls` prepends `https://`, so hand it the
            // bare host — strip any scheme / trailing slash from cfg.url.
            let domain = cfg
                .url
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .trim_end_matches('/');
            let client = db_clickhouse::mtls::client_with_mtls(domain, &bundle, &cfg.database)
                .unwrap_or_else(|e| panic!("mTLS client build failed: {e}"));
            sink::Sink::new(client)
        }
        (None, None, None) => sink::Sink::new(db_clickhouse::client(&cfg)),
        _ => {
            panic!("--ch-cert / --ch-key / --ch-ca must all be set together (mTLS) or all omitted")
        }
    }
}
