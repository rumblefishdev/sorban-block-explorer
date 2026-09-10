//! The checkpoint-snapshot seed (task 0463 step 3d, ADR 0055) — build every
//! correction the four-way comparison proved necessary, as INSERT-ready rows.
//!
//! ## What it writes (with `--execute`; without it, artifacts + counts only)
//!
//! | correction | rows measured 2026-08-18 | version ledger | closed_at |
//! |---|---|---|---|
//! | missing live holding | ~19.3M classic | the ENTRY's own `lastModifiedLedgerSeq` | 0 |
//! | closure (ours 0, gone) | ~22.2M classic + 2.3M native | checkpoint | checkpoint |
//! | ghost (ours >0, gone) | ~1.04M native + ~2k classic | checkpoint | checkpoint |
//! | self-heal (snapshot newer) | ~25k | the entry's own ledger | 0 |
//! | `account_entry_state` | live accounts newer than what we hold (0521) | the entry's own ledger | — |
//! | `assets` / `accounts` dimension stubs | the referenced ids we lack | entry ledger | — |
//!
//! ## The versioning contract (the load-bearing part)
//!
//! - **Live data always versions on the entry's own `lastModifiedLedgerSeq`**,
//!   never a window boundary — the task 0492 defect. The live parser's newer
//!   writes then win regardless of load order.
//! - **Closures version on the checkpoint ledger.** There is no entry left to
//!   carry a ledger, and the checkpoint IS the ledger of the observation
//!   "this is gone". `closed_at_ledger` gets the same value and therefore
//!   means "closed AT OR BEFORE this ledger" for seeded rows (the writer's own
//!   stamps are exact). Side effect: every seed-stamped closure shares the run's
//!   checkpoint value — a recognisable cohort, which is the only provenance we
//!   have until task 0492 lands a real convention.
//! - **Ghosts (positive-but-absent) are corrected AND reported, never silent.**
//!   The standing policy said "anomaly report, not silent closure" back when
//!   these were presumed ingestion gaps; RPC verification (100/100 sampled
//!   accounts ABSENT on chain, amounts byte-checked) proved them real
//!   removals, so option A applies — `amount = 0` together with
//!   `closed_at_ledger` — but the full list still lands in the artifacts dir.
//!
//! ## Deployment order — DO NOT reorder
//!
//! 1. Deploy the lifecycle writer (stage.rs stamps closures itself).
//! 2. THEN run this seed against a checkpoint taken AFTER that deploy.
//!
//! Reversed, every removal between the checkpoint and the deploy is written by
//! the OLD writer as a plain `amount = 0, closed_at = 0` row with a ledger
//! ABOVE the checkpoint — it outversions the seed's closure and resurrects the
//! ghost. The 12-minute window of task 0310 taught the same lesson.
//!
//! ## What is deliberately NOT seeded here
//!
//! - **Pool-share trustlines** (77,048 live) — they live in `lp_positions`
//!   until the ADR 0056 merge lands. The archive is content-addressed (bucket
//!   files keyed by hash), so the LP pass re-derives this snapshot from the
//!   manifest JSON saved into the artifacts dir; nothing else must be kept.
//! - **Type-3 / contract-held holdings** — different ledger entry type
//!   (`ContractData`); their audit is task 0503 on the same decoder.
//!
//! ## Why the `assets` stubs need no RMT version
//!
//! `assets` is `ReplacingMergeTree` with NO version column, keyed on the
//! identity 4-tuple — on merge ClickHouse keeps the last-inserted row per
//! key. That is safe here because every field of an `AssetRow` (including the
//! `id` surrogate) is a pure function of that same identity tuple: any two
//! rows ever written for one key are byte-identical, so it cannot matter
//! which survives. Stubs are additionally emitted only for ids absent from
//! the known-id set read from ClickHouse, so they never contend with an existing row.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::error::BackfillError;
use crate::sink::Sink;
use crate::snapshot::archive::PUBNET_ARCHIVE;
use crate::snapshot::network_state::{self, NetworkState};
use crate::snapshot::report::Report;
use crate::snapshot::verdict;
use crate::util::insert_rows;
use db_clickhouse::persist::ids;
use db_clickhouse::persist::rows::{AccountEntryStateRow, AccountRow, AssetRow, BalanceRow};

/// Insert batch size. RowBinary streams; this only bounds peak buffering.
const INSERT_CHUNK: usize = 500_000;

/// All correction rows, still in memory, ready to insert or count.
///
/// **Deliberately materialised rather than streamed.** A dry-run over the full
/// network measured 4.5 GB peak RSS for ~44.9M balance rows plus 10.9M signer
/// rows — comfortable on the operator box, and holding them lets the run
/// report exact counts and write `summary.txt` BEFORE any insert, which is the
/// property that makes `--execute` reviewable. Streaming inserts per batch
/// would halve the memory but would start writing before the totals are known.
/// If this ever needs to run somewhere smaller, stream passes 1/2/4 and keep
/// only the counters — the insert ORDER (dimension stubs before the balances
/// that reference them) must be preserved.
#[derive(Default)]
struct Corrections {
    balances: Vec<BalanceRow>,
    entry_states: Vec<AccountEntryStateRow>,
    /// Live accounts pass 4 skipped because we already hold their state at that
    /// ledger or later. Reported beside `entry_states`, and the two sum to the
    /// snapshot's live-account count — the invariant the summary's arithmetic
    /// is checked against.
    entry_states_unchanged: u64,
    asset_stubs: Vec<AssetRow>,
    account_stubs: Vec<AccountRow>,
    /// One line per row this run zeroes while it still held a positive amount
    /// — the anomaly report, and the only pre-image of what `--execute` takes
    /// away. It belongs to what the run produced, like the four row sets.
    ghosts: Vec<String>,
    dangling: Dangling,
}

/// References a seeded row introduces that resolve to no dimension row —
/// neither already in ClickHouse nor stubbed by this run.
///
/// The three sites that produce these were silent `continue`s: an id the
/// snapshot references but carries no identity for is simply not stubbed, and
/// the balance pointing at it goes in anyway. That is the failure mode the
/// stub pass exists to prevent, so it is counted, reported, and (for the two
/// that break a balance) refused at `--execute` rather than skipped.
#[derive(Default)]
struct Dangling {
    /// A seeded balance points at this asset; nothing will define it.
    assets: u64,
    /// A seeded balance belongs to this holder; nothing will define it.
    holders: u64,
    /// An asset stub names this issuer; nothing will define it. Cosmetic — it
    /// blanks the issuer column, it never orphans a balance — and legitimate:
    /// an issuer may merge while trustlines to its asset outlive it.
    issuers: u64,
}

/// Number of `holder_id` slices. 64 keeps each chunk near the ~760k groups
/// measured for 1/64 of the key space — two orders under the server's limit.
const KEY_SLICES: i128 = 64;

/// Floor on the our-rows read. A short
/// read (wrong database, a dropped key slice) is indistinguishable from a real
/// one downstream: every missing row becomes an unmatched snapshot entry, i.e.
/// a phantom network gap the seed would INSERT as a live holding. The real
/// population measured 48.6M distinct (holder, asset) pairs — sit just under.
pub(crate) const MIN_OUR_ROWS: u64 = 40_000_000;

/// Stream our deduplicated `balances` in `holder_id` slices, invoking `f` per
/// row. Like every
/// other corrective command in this crate, the tool reads its own inputs
/// through `sink.client()`; there is no manual export step. (A hand-exported
/// TSV transport existed during the research phase and was removed with the
/// 2026-08-21 self-read decision: the binary holds the same connection for `--execute`
/// inserts anyway, and a cursor error propagates loudly where the operator
/// CLI's exit-0-on-server-error trap did not.)
///
/// Errors below [`MIN_OUR_ROWS`] rows: a short read (wrong database,
/// dropped slice) would silently report our own holdings as a phantom network
/// gap.
async fn stream_our_rows(
    sink: &Sink,
    mut f: impl FnMut(&verdict::OurRow),
) -> Result<u64, BackfillError> {
    let mut seen = 0u64;
    for (i, (from, to)) in key_slices().enumerate() {
        // `argMax` collapses the ReplacingMergeTree duplicates the way a read
        // must: prod tables carry unmerged parts, so a plain SELECT double-counts.
        let mut cursor = sink
            .client()
            .query(&slice_sql(from, to))
            .fetch::<verdict::OurRow>()?;
        while let Some(row) = cursor.next().await? {
            seen += 1;
            f(&row);
        }
        println!("    slice {:>2}/{KEY_SLICES} — {seen} rows so far", i + 1);
    }
    if seen < MIN_OUR_ROWS {
        return Err(BackfillError::Incomplete(format!(
            "our balances read returned {seen} rows, expected at least {} — a short \
             read reports our own holdings as a phantom network gap (wrong database?)",
            MIN_OUR_ROWS
        )));
    }
    Ok(seen)
}

/// The per-slice read. Its SELECT list is aliased to the FIELD NAMES of
/// [`verdict::OurRow`], which is what the driver matches on: `clickhouse` 0.15
/// builds a name-to-field mapping per cursor and returns `SchemaMismatch` on a
/// count mismatch or an unknown column, so a renamed alias fails the query
/// rather than shifting a column silently. Decoding is not positional — an
/// earlier version of this comment said it was, and two reviews built findings
/// on that sentence.
fn slice_sql(from: i128, to: i128) -> String {
    // The aggregates are aliased INSIDE a subquery and renamed outside. Aliasing
    // `max(last_updated_ledger) AS last_updated_ledger` directly shadows the
    // column, so the next `argMax(..., last_updated_ledger)` binds the alias and
    // ClickHouse rejects it: "Aggregate function max(...) is found inside
    // another aggregate function" (ILLEGAL_AGGREGATION).
    //
    // ONE `argMax` over a TUPLE, not one per column. Two independent `argMax`
    // aggregates resolve a same-version tie independently, so in principle they
    // could take `amount` from one row and `closed_at_ledger` from another and
    // hand back a row that exists in no part on disk. Measured, they do not:
    // over a full key slice (762,955 keys, 19,142 ties argMax actually has to
    // resolve) the two forms returned identical results, because ClickHouse
    // keeps the first-encountered maximum and both states walk the same rows in
    // the same order. That is an implementation property, not a contract, and
    // the tuple costs nothing — so the guarantee is structural instead.
    //
    // Not a repair of the 1,238,583 known ties: every one of those carries
    // `closed_at_ledger = 0` on BOTH sides (they predate the column), so no
    // assembly of them can differ from a real row. This closes the shape a
    // FUTURE tie could take, once the deployed writer's stamps start appearing
    // at contended versions.
    format!(
        "SELECT holder_id, asset_id, tupleElement(best, 1) AS amount, \
                led AS last_updated_ledger, tupleElement(best, 2) AS closed_at_ledger \
         FROM ( \
             SELECT holder_id, \
                    asset_id, \
                    argMax((amount, closed_at_ledger), last_updated_ledger) AS best, \
                    max(last_updated_ledger) AS led \
             FROM balances \
             WHERE holder_id BETWEEN {from} AND {to} \
               AND asset_id IN (SELECT id FROM assets WHERE asset_type IN (0, 1)) \
               AND holder_id NOT IN (SELECT id FROM soroban_contracts) \
             GROUP BY holder_id, asset_id \
         )"
    )
}

/// The slice boundaries, covering the i64 key space exactly once.
fn key_slices() -> impl Iterator<Item = (i128, i128)> {
    let lo = i128::from(i64::MIN);
    let hi = i128::from(i64::MAX);
    let step = (hi - lo + 1) / KEY_SLICES;
    (0..KEY_SLICES).map(move |s| {
        let from = lo + s * step;
        let to = if s == KEY_SLICES - 1 {
            hi
        } else {
            lo + (s + 1) * step - 1
        };
        (from, to)
    })
}

/// Fetch the set of existing dimension ids straight from ClickHouse — the
/// tool reads its own inputs, like every other corrective command here.
///
/// Sliced on `id`, for the same reason [`stream_our_rows`] is. The unsliced
/// version read `accounts` in one query and timed out on the third production
/// run: 14.58M ids do not fit the operator profile's `max_execution_time = 30`,
/// which counts the time spent SENDING rows, not just aggregating them (the
/// aggregation alone measures 0.4s). It had succeeded twice before that, which
/// is the worst kind of limit — one that depends on how busy the server is.
///
/// Failure was loud, and stayed loud: a cursor error propagates, so a partial
/// id set can never be mistaken for a real one. That matters more here than
/// almost anywhere else in the run, because fewer known ids means more ids
/// judged absent, which means more dimension stubs — a truncated read would
/// manufacture rows for entities that already exist.
async fn fetch_id_set(sink: &Sink, table: &str) -> Result<HashSet<i64>, BackfillError> {
    #[derive(clickhouse::Row, serde::Deserialize)]
    struct IdRow {
        id: i64,
    }
    let mut out = HashSet::new();
    for (from, to) in key_slices() {
        // `GROUP BY id` collapses the RMT duplicates prod tables carry
        // unmerged. The HashSet would dedup anyway; doing it server-side keeps
        // ~8% of rows off the wire, which is the resource actually constrained.
        let sql = format!("SELECT id FROM {table} WHERE id BETWEEN {from} AND {to} GROUP BY id");
        let mut cursor = sink.client().query(&sql).fetch::<IdRow>()?;
        while let Some(r) = cursor.next().await? {
            out.insert(r.id);
        }
    }
    Ok(out)
}

/// The version we already hold per account in `account_entry_state` — the
/// input pass 4 compares against, so it emits what CHANGED rather than the
/// whole live-account set.
///
/// `max`, not `argMax` over a tuple: only the version is compared here. The
/// content diff at an EQUAL version is a real defect class (a writer that put
/// wrong signers at the right ledger) and it is 0503's, not this pass's — it
/// costs three arrays per account over the wire, and it is an audit rather
/// than a gate on a load path.
///
/// Sliced on `account_id` for [`fetch_id_set`]'s reason: `max_execution_time`
/// counts the time spent SENDING rows and the ceiling is load-dependent. The
/// slices are far from equal — measured 85,229 accounts in the first of 64
/// against 10,896,350 total — but even the fat ones sit well under the ~760k
/// groups the balances read already carries per slice.
///
/// Costs ~260 MB of the run's ~4.5 GB peak RSS (10.9M entries at 16 B plus
/// hashing overhead), which is why it is a version map and not the rows.
///
/// **No floor, deliberately** — and this is the one input here that should not
/// have one. Every other read in this tool carries a minimum because a short
/// read manufactures rows for entities that already exist. This one fails the
/// opposite way: fewer known versions means MORE rows emitted, and they are
/// byte-identical at the same RMT version, so the data is unaffected and only
/// the "unchanged" count reads low. A floor would additionally refuse the
/// legitimate empty case — the first seed, or a fresh local database.
async fn fetch_entry_state_versions(sink: &Sink) -> Result<HashMap<i64, i64>, BackfillError> {
    #[derive(clickhouse::Row, serde::Deserialize)]
    struct VersionRow {
        account_id: i64,
        led: i64,
    }
    let mut out = HashMap::new();
    for (from, to) in key_slices() {
        // `GROUP BY account_id` collapses the unmerged ReplacingMergeTree
        // parts production carries — a plain SELECT returns the same account
        // several times at different versions.
        let sql = format!(
            "SELECT account_id, max(last_updated_ledger) AS led \
             FROM account_entry_state \
             WHERE account_id BETWEEN {from} AND {to} \
             GROUP BY account_id"
        );
        let mut cursor = sink.client().query(&sql).fetch::<VersionRow>()?;
        while let Some(r) = cursor.next().await? {
            out.insert(r.account_id, r.led);
        }
    }
    Ok(out)
}

/// Does the snapshot's `AccountEntry` at `snapshot_ledger` say anything we do
/// not already hold?
///
/// `ours >= snapshot_ledger` skips both ways it can happen. EQUAL means the two
/// describe the same `AccountEntry`, so the write is a no-op. GREATER means the
/// live writer is ahead of the checkpoint — the same reasoning the balances
/// path applies with its `>= checkpoint` guard, and the reason a seed row could
/// never win there either.
fn entry_state_is_new(ours: Option<i64>, snapshot_ledger: i64) -> bool {
    match ours {
        Some(held) => held < snapshot_ledger,
        None => true,
    }
}

/// Emit the correction one verdict implies. The verdict comes from the REPORT,
/// which counted and sampled the same row a moment earlier — so the summary an
/// operator signs off on and the rows `--execute` writes are derived from one
/// classification, not two.
///
/// Within ONE run. Across two runs the populations differ, and the honest
/// statement of how is worth spelling out, because an earlier version of this
/// comment claimed more than it could:
///
/// - OUR side drifts harmlessly. `--execute` re-reads our rows fresh, like
///   every corrective command here, and anything the live writer touched since
///   is absorbed by the `>= checkpoint` guard — those rows are newly LEFT
///   ALONE, never given a different correction.
/// - The SNAPSHOT side drifts too, and that half the old comment did not
///   reason about. Checkpoints publish every 64 ledgers (~5 minutes) while a
///   full pass takes ~5 (measured: 317 s dry-run, 637 s with the inserts;
///   the archive download dominates and is network-bound, so earlier runs
///   measured 909 s on the same code). What separates two runs is therefore a
///   whole pass PLUS the operator reading `summary.txt`, which is why
///   `--execute` still always decodes a later checkpoint than the dry-run
///   reviewed — but the margin is one checkpoint interval, not three.
///   Holdings the network
///   created in that window are `missing` in the second run and get INSERTED,
///   without having appeared in the summary an operator signed off on.
///
/// That drift is accepted deliberately (2026-08-21, reaffirmed 2026-08-26).
/// The rows it adds are real live holdings — the fresher snapshot is the
/// better input, not a riskier one — and the run is verified by measuring its
/// OUTCOME against the network (coverage, the 200-account chain probe,
/// aggregate deltas), which a frozen input would not improve. `manifest.json`
/// records the checkpoint each run actually used, so the population is always
/// identifiable after the fact.
fn fold_our_row(
    row: &verdict::OurRow,
    verdict: verdict::Verdict,
    net: Option<network_state::NetHolding>,
    checkpoint: u32,
    out: &mut Corrections,
) {
    use verdict::Verdict as V;
    if verdict == V::Ghost {
        out.ghosts.push(format!(
            "{}\t{}\t{}\t{}",
            row.holder_id, row.asset_id, row.amount, row.last_updated_ledger
        ));
    }
    let Some(c) = verdict::correction(verdict, net.as_ref(), checkpoint) else {
        return;
    };
    out.balances.push(BalanceRow {
        holder_id: row.holder_id,
        asset_id: row.asset_id,
        amount: c.amount,
        last_updated_ledger: c.last_updated_ledger,
        closed_at_ledger: c.closed_at_ledger,
    });
}

/// Build every correction. Deterministic function of (snapshot state, our
/// rows as read, dimension id sets) — a re-run against the same inputs
/// produces identical rows and RMT collapses them.
async fn build_corrections(
    sink: &Sink,
    state: &mut NetworkState,
    known_assets: &HashSet<i64>,
    known_accounts: &HashSet<i64>,
    checkpoint: u32,
    report: &mut Report,
) -> Result<Corrections, BackfillError> {
    let mut out = Corrections::default();

    // Pass 1: our rows → the report classifies, counts and samples; the verdict
    // it hands back drives the correction. One classification, two outputs.
    println!("\n  streaming our balances in {KEY_SLICES} key slices…");
    let rows_read = stream_our_rows(sink, |row| {
        let (v, net) = report.observe(row, state);
        fold_our_row(row, v, net, checkpoint, &mut out);
    })
    .await?;
    println!("  folded {rows_read} of our rows");

    // Pass 2: unmatched live snapshot entries → missing-holding inserts.
    let mut referenced_assets: HashSet<i64> = HashSet::new();
    let mut referenced_holders: HashSet<i64> = HashSet::new();
    for (key, e) in &state.trustlines {
        if e.live && !e.matched {
            report.observe_missing_trustline(key, e, state);
            out.balances.push(BalanceRow {
                holder_id: key.holder_id,
                asset_id: key.asset_id,
                amount: i128::from(e.balance),
                last_updated_ledger: i64::from(e.ledger),
                closed_at_ledger: 0,
            });
            referenced_assets.insert(key.asset_id);
            referenced_holders.insert(key.holder_id);
        }
    }
    for (id, e) in &state.accounts {
        if e.live && !e.matched {
            report.observe_missing_account(*id, e, state);
            out.balances.push(BalanceRow {
                holder_id: *id,
                asset_id: ids::NATIVE_ASSET_ID,
                amount: i128::from(e.balance),
                last_updated_ledger: i64::from(e.ledger),
                closed_at_ledger: 0,
            });
            referenced_holders.insert(*id);
        }
    }

    // Pass 3: dimension stubs — a seeded balance whose asset or holder has no
    // dimension row would render as a broken join, i.e. a new lie replacing an
    // old one. Issuers of stubbed assets count as referenced accounts too.
    // Issuers are kept apart from balance holders: a missing issuer stub blanks
    // a column, a missing holder stub orphans a balance. Only the second is a
    // reason to refuse the write, so they cannot share a counter.
    let mut referenced_issuers: HashSet<i64> = HashSet::new();
    for asset_id in &referenced_assets {
        if known_assets.contains(asset_id) {
            continue;
        }
        let Some((code, issuer)) = state.asset_registry.get(asset_id) else {
            // The snapshot references this asset but carries no live trustline
            // to recover its (code, issuer) from — so the balance above points
            // at an `assets` row that will never exist.
            out.dangling.assets += 1;
            continue;
        };
        let issuer_id = ids::account_id(issuer);
        out.asset_stubs.push(AssetRow {
            asset_type: 1,
            asset_code: code.clone(),
            issuer_id,
            contract_id: 0,
            id: *asset_id,
        });
        referenced_issuers.insert(issuer_id);
    }
    // `union`, not `chain`: an issuer that also holds a balance appears in both
    // sets and would otherwise be stubbed twice.
    for holder_id in referenced_holders.union(&referenced_issuers) {
        if known_accounts.contains(holder_id) {
            continue;
        }
        let is_balance_holder = referenced_holders.contains(holder_id);
        let unresolved = |d: &mut Dangling| {
            if is_balance_holder {
                d.holders += 1;
            } else {
                d.issuers += 1;
            }
        };
        let Some(d) = state.account_details.get(holder_id) else {
            unresolved(&mut out.dangling); // referenced, not a live snapshot account
            continue;
        };
        // `get`, not `[]`: safe today only because `absorb` fills `accounts`
        // and `account_details` together, which is an invariant rather than a
        // type guarantee — and this path runs after 4.5 GB of decode.
        let Some(entry) = state.accounts.get(holder_id).copied() else {
            unresolved(&mut out.dangling);
            continue;
        };
        out.account_stubs.push(AccountRow {
            id: *holder_id,
            account_id: d.strkey.clone(),
            // The entry's lastModified is an UPPER BOUND on creation; the true
            // first-seen predates our history. Better than a fabricated 0.
            first_seen_ledger: i64::from(entry.ledger),
            last_seen_ledger: i64::from(entry.ledger),
            sequence_number: d.seq_num,
            home_domain: (!d.home_domain.is_empty()).then(|| d.home_domain.clone()),
        });
    }

    // Pass 4: entry state — signers, thresholds and flags for every live
    // account whose snapshot ledger is ABOVE what we already hold, versioned on
    // the entry's own ledger so the live writer wins on any later change.
    //
    // It emitted the full live-account set unconditionally until 0521. That was
    // right when it was written — the live signers writer did not exist yet, so
    // the whole set genuinely WAS the correction — and wrong from the moment
    // that writer deployed (2026-08-24). Measured on the second production pass
    // (S1, checkpoint 64,132,415): every other number collapsed to the churn it
    // should be (`balances` corrections 44,834,785 -> 1, `missing` and the stubs
    // to 0) while this one re-emitted 10,872,072 rows. The data was never at
    // risk — the rows are byte-identical at the same version and RMT collapses
    // them — but a number that cannot move tells its reader nothing, and it
    // occupies the slot where a live-writer regression would show up.
    println!("\n  reading the entry-state versions we already hold…");
    let held = fetch_entry_state_versions(sink).await?;
    println!("  hold {} accounts", held.len());
    for (id, e) in &state.accounts {
        if !e.live {
            continue;
        }
        let Some(d) = state.account_details.get(id) else {
            continue;
        };
        if !entry_state_is_new(held.get(id).copied(), i64::from(e.ledger)) {
            out.entry_states_unchanged += 1;
            continue;
        }
        out.entry_states.push(AccountEntryStateRow {
            account_id: *id,
            signer_keys: d.signers.iter().map(|(k, _, _)| k.clone()).collect(),
            signer_weights: d.signers.iter().map(|(_, w, _)| *w).collect(),
            signer_types: d.signers.iter().map(|(_, _, t)| t.to_string()).collect(),
            master_weight: d.thresholds[0],
            threshold_low: d.thresholds[1],
            threshold_med: d.thresholds[2],
            threshold_high: d.thresholds[3],
            flags: d.flags,
            last_updated_ledger: i64::from(e.ledger),
        });
    }

    Ok(out)
}

/// Cap on the two row sets too large to dump whole. Truncation is always
/// stated in the file itself — a dump that silently stops reads as a complete
/// one to whoever audits it.
const DUMP_CAP: usize = 5_000;

/// Write `lines` to `dir/name`, capped, with the cut recorded in the file.
fn write_dump(
    dir: &Path,
    name: &str,
    total: usize,
    lines: impl Iterator<Item = String>,
) -> Result<(), BackfillError> {
    let mut out: Vec<String> = lines.collect();
    if total > out.len() {
        out.push(format!("# TRUNCATED — {} of {total} rows shown", out.len()));
    }
    let path = dir.join(name);
    std::fs::write(&path, out.join("\n") + "\n")
        .map_err(|e| BackfillError::Incomplete(format!("write {}: {e}", path.display())))?;
    println!(
        "    wrote {} of {total} rows -> {}",
        out.len(),
        path.display()
    );
    Ok(())
}

/// Dump the three row sets the verdict samples never covered: the ones that
/// invent an ENTITY rather than restate a holding. A wrong amount on a real
/// asset is visible to anyone who looks the asset up; an asset that does not
/// exist on chain is not, because nobody knows to look for it. Asset stubs are
/// therefore dumped WHOLE — they are the smallest set and the only one that
/// writes a new row into a dimension table.
///
/// The two capped dumps take an ARBITRARY prefix, not the deterministic
/// bottom-k the verdict samples use: both vectors are built by iterating a
/// `HashMap`, whose order is per-process, so these files are not comparable
/// across runs. They exist to be eyeballed and chain-checked, not diffed.
fn write_correction_dumps(
    dir: &Path,
    corr: &Corrections,
    state: &NetworkState,
) -> Result<(), BackfillError> {
    write_dump(
        dir,
        "asset_stubs.tsv",
        corr.asset_stubs.len(),
        corr.asset_stubs.iter().map(|a| {
            // The registry issuer is the StrKey the surrogate was derived from;
            // printing both lets an audit recompute `credit_asset_id` offline.
            let issuer = state
                .asset_registry
                .get(&a.id)
                .map_or("?", |(_, issuer)| issuer.as_str());
            format!("{}\t{}\t{}\t{}", a.asset_code, issuer, a.id, a.issuer_id)
        }),
    )?;
    write_dump(
        dir,
        "account_stubs.tsv",
        corr.account_stubs.len(),
        corr.account_stubs
            .iter()
            .take(DUMP_CAP)
            .map(|a| format!("{}\t{}\t{}", a.account_id, a.id, a.first_seen_ledger)),
    )?;
    write_dump(
        dir,
        "entry_states.tsv",
        corr.entry_states.len(),
        corr.entry_states.iter().take(DUMP_CAP).map(|s| {
            // The StrKey, not the surrogate: a signer set is audited by asking
            // the chain for the account, which needs the G-address.
            let who = state
                .account_details
                .get(&s.account_id)
                .map_or("?", |d| d.strkey.as_str());
            format!(
                "{who}\t{}/{}/{}/{}\t{}\t{}",
                s.master_weight,
                s.threshold_low,
                s.threshold_med,
                s.threshold_high,
                s.signer_keys.join(","),
                s.signer_weights
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }),
    )
}

async fn insert_chunked<T>(sink: &Sink, table: &str, rows: &[T]) -> Result<(), BackfillError>
where
    T: clickhouse::Row + clickhouse::RowOwned + serde::Serialize,
{
    for chunk in rows.chunks(INSERT_CHUNK) {
        insert_rows(sink.client(), table, chunk).await?;
    }
    Ok(())
}

/// Refuse `--execute` under a read-only identity, BEFORE the run costs
/// anything.
///
/// The failure it prevents is not subtle, it is just late: the first INSERT is
/// the last step of the pass, so a wrong certificate spends ~5 minutes and
/// 4.4 GB of archive download to learn a fact one SELECT already knows. The
/// operator laptop's cert maps to a `readonly = 1` user, which refuses an
/// INSERT before grants are consulted at all, so this is the ordinary case and
/// not an exotic one.
///
/// A READ, never a trial write: the decisive test for a write permission must
/// not itself be a write. `readonly` is the setting that actually gates the
/// insert — grants are never reached while it is 1 — so it is what gets
/// checked, rather than the user name, which is only reported to make the
/// message actionable.
async fn refuse_if_read_only(sink: &Sink) -> Result<(), BackfillError> {
    let (user, readonly): (String, u8) = sink
        .client()
        .query("SELECT currentUser(), toUInt8(getSetting('readonly'))")
        .fetch_one()
        .await?;
    if readonly != 0 {
        return Err(BackfillError::Incomplete(format!(
            "refusing --execute: connected as `{user}` with readonly = {readonly}, which \
             rejects every INSERT before grants are consulted. Use a write-capable \
             identity (see the write-identity section of docs/backfills.md); the \
             dry-run needs no change."
        )));
    }
    println!("  identity: {user} (readonly = 0) — writes permitted");
    Ok(())
}

/// The seed. Without `--execute`: reads its inputs from ClickHouse, decodes
/// the snapshot, folds, writes artifacts, inserts NOTHING. With `--execute`:
/// additionally inserts the four row sets.
pub async fn seed_command(
    sink: &Sink,
    artifacts_root: &Path,
    execute: bool,
) -> Result<(), BackfillError> {
    let started = std::time::Instant::now();

    // Before anything expensive: a write we cannot make is worth knowing in
    // second 2, not in minute 11.
    if execute {
        refuse_if_read_only(sink).await?;
    }

    let (list, mut state, source_report) =
        network_state::open_snapshot(if execute { " [EXECUTE]" } else { " [dry-run]" }).await?;

    // One directory per checkpoint, so a run never overwrites the record of an
    // earlier one — `ghosts.tsv` is the only pre-image of what a run zeroed.
    let artifacts = &artifacts_root.join(list.checkpoint_ledger.to_string());
    std::fs::create_dir_all(artifacts)
        .map_err(|e| BackfillError::Incomplete(format!("mkdir {}: {e}", artifacts.display())))?;
    println!("  artifacts → {}", artifacts.display());

    // Provenance artifact: the exact bucket list this run decoded. The archive
    // is content-addressed, so this manifest alone identifies the identical
    // snapshot later (the LP-merge pass will need exactly that).
    let manifest = serde_json::json!({
        "checkpoint_ledger": list.checkpoint_ledger,
        "archive": PUBNET_ARCHIVE,
        "buckets": list.hashes,
    });
    std::fs::write(
        artifacts.join("manifest.json"),
        serde_json::to_string_pretty(&manifest).expect("static json"),
    )
    .map_err(|e| BackfillError::Incomplete(format!("write manifest: {e}")))?;

    let known_assets = fetch_id_set(sink, "assets").await?;
    let known_accounts = fetch_id_set(sink, "accounts").await?;
    println!(
        "  known dimension ids: {} assets, {} accounts",
        known_assets.len(),
        known_accounts.len()
    );
    // A short read is indistinguishable from a real one to everything
    // downstream: fewer known ids means more "absent" ids means more stubs.
    // These floors are far below the measured populations (344,989 assets /
    // 14.5M accounts as of 2026-08-18) — they catch a wrong database, not a
    // shrinking network.
    const MIN_ASSET_IDS: usize = 100_000;
    const MIN_ACCOUNT_IDS: usize = 5_000_000;
    if known_assets.len() < MIN_ASSET_IDS || known_accounts.len() < MIN_ACCOUNT_IDS {
        return Err(BackfillError::Incomplete(format!(
            "dimension id read looks wrong ({} assets, {} accounts; expected at least \
             {MIN_ASSET_IDS} and {MIN_ACCOUNT_IDS}) — is this the production database?",
            known_assets.len(),
            known_accounts.len()
        )));
    }

    let mut report = Report::new(list.checkpoint_ledger);
    let corr = build_corrections(
        sink,
        &mut state,
        &known_assets,
        &known_accounts,
        list.checkpoint_ledger,
        &mut report,
    )
    .await?;

    // The ghost list is the anomaly REPORT the policy demands — corrected in
    // the same run, but never silently.
    std::fs::write(artifacts.join("ghosts.tsv"), corr.ghosts.join("\n") + "\n")
        .map_err(|e| BackfillError::Incomplete(format!("write ghosts: {e}")))?;

    // The summary IS the four-way comparison — the same twelve buckets per
    // population the report renders, from one `Report`, plus
    // what this run would insert. An operator signs off on one document.
    report.write_dumps(&artifacts.join("dumps"))?;
    write_correction_dumps(&artifacts.join("dumps"), &corr, &state)?;
    // Excluded on purpose — reported so the pass never reads as exhaustive
    // when it is not. Contract-held classic balances live in the SAC's
    // `ContractData`, not a trustline, so the snapshot's trustline set would
    // call every one of them a phantom; type-3 is the same reason, different
    // entry type; pool shares are the same ledger entry type but live in
    // `lp_positions` on our side (ADR 0056 merges them).
    // `uniqExact`, not `count()`: production tables carry unmerged
    // ReplacingMergeTree parts, so a raw row count is 2-3x the number of
    // holdings (measured 182,370 rows over 70,347 keys). The compared
    // population is counted per KEY — see `slice_sql`'s GROUP BY — and a
    // report that mixes the two bases invites exactly the comparison its
    // reader will make.
    let excluded_contract: u64 = sink
        .client()
        .query(
            "SELECT uniqExact((holder_id, asset_id)) FROM balances \
             WHERE asset_id IN (SELECT id FROM assets WHERE asset_type IN (0, 1)) \
               AND holder_id IN (SELECT id FROM soroban_contracts)",
        )
        .fetch_one()
        .await?;
    let excluded_type3: u64 = sink
        .client()
        .query(
            "SELECT uniqExact((holder_id, asset_id)) FROM balances \
             WHERE asset_id IN (SELECT id FROM assets WHERE asset_type = 3)",
        )
        .fetch_one()
        .await?;

    let summary = format!(
        "checkpoint {}\n{}{}{}{}\n  NOT COMPARED (deliberate, see module docs)\n    \
         contract-held classic rows  {:>12}\n    \
         type-3 Soroban rows         {:>12}\n    \
         snapshot pool shares        {:>12}  (our side: lp_positions)\n\
         \n  CORRECTIONS{}\n    \
         balances rows         {:>12}\n    \
         account_entry_state   {:>12}  ({} unchanged, already held at that ledger or later)\n    \
         asset stubs           {:>12}\n    \
         account stubs         {:>12}\n\
         \n  UNRESOLVED REFERENCES (must be 0 for the first two)\n    \
         assets a seeded balance points at  {:>12}\n    \
         holders a seeded balance is for    {:>12}\n    \
         issuers an asset stub names        {:>12}  (blanks a column, never a balance)\n\
         \n  ghosts.tsv holds every positive-amount row this run zeroes.\n  \
         dumps/asset_stubs.tsv holds EVERY new assets row, for offline audit.\n",
        list.checkpoint_ledger,
        source_report,
        report.classic.render("CLASSIC CREDIT trustlines", false),
        report
            .native
            .render("NATIVE XLM holdings (AccountEntry, not a trustline)", true),
        report.render_missing_histogram(),
        excluded_contract,
        excluded_type3,
        state.live_pool_shares(),
        if execute {
            " — INSERTING"
        } else {
            " — dry-run, nothing inserted"
        },
        corr.balances.len(),
        corr.entry_states.len(),
        corr.entry_states_unchanged,
        corr.asset_stubs.len(),
        corr.account_stubs.len(),
        corr.dangling.assets,
        corr.dangling.holders,
        corr.dangling.issuers,
    );
    std::fs::write(artifacts.join("summary.txt"), &summary)
        .map_err(|e| BackfillError::Incomplete(format!("write summary: {e}")))?;
    println!("\n{summary}");

    if execute {
        // A balance whose asset or holder resolves to no dimension row renders
        // as a broken join — the "new lie replacing an old one" the stub pass
        // exists to prevent. The dry-run reports it; the write refuses it.
        if corr.dangling.assets > 0 || corr.dangling.holders > 0 {
            return Err(BackfillError::Incomplete(format!(
                "refusing to insert: {} seeded balances point at an undefined asset and {} at \
                 an undefined holder — every such row would render as a broken join. See the \
                 UNRESOLVED REFERENCES block in summary.txt",
                corr.dangling.assets, corr.dangling.holders
            )));
        }
        println!("  inserting…");
        insert_chunked(sink, "assets", &corr.asset_stubs).await?;
        insert_chunked(sink, "accounts", &corr.account_stubs).await?;
        insert_chunked(sink, "balances", &corr.balances).await?;
        insert_chunked(sink, "account_entry_state", &corr.entry_states).await?;
        println!("  inserts done.");
    } else {
        println!("  dry-run: nothing inserted. Re-run with --execute to write.");
    }
    println!("  total {:.1}s", started.elapsed().as_secs_f64());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The gate pass 4 gained in 0521. Its whole job is to be false on a second
    /// pass, so the case that matters most is the middle one: a row we hold at
    /// EXACTLY the snapshot's ledger is the same `AccountEntry`, and re-writing
    /// it is the 10.9M-row no-op this gate exists to stop.
    #[test]
    fn entry_state_gate_emits_only_what_is_newer_than_what_we_hold() {
        // Never seen — the first seed, and every account created since.
        assert!(entry_state_is_new(None, 64_131_263));
        // We are behind: the account changed after our last row.
        assert!(entry_state_is_new(Some(64_000_000), 64_131_263));
        // Same ledger, same entry — a write that changes nothing.
        assert!(!entry_state_is_new(Some(64_131_263), 64_131_263));
        // The live writer is ahead of the checkpoint; a seed row would lose the
        // ReplacingMergeTree comparison anyway.
        assert!(!entry_state_is_new(Some(64_140_000), 64_131_263));
    }
}
