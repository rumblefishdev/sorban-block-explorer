//! ClickHouse queries for the accounts endpoints (task 0243).
//!
//! Returns the `AccountHeaderRow` / `AccountBalanceRow` / `AccountTxRow`
//! shapes, so the handler stays backend-agnostic after the fetch. Notable
//! CH translation choices:
//!
//! - **`asset_type_name()` is a PG SQL function.** CH has no equivalent, so
//!   the `asset_type` → label mapping is done in Rust ([`asset_type_name`]),
//!   identical to the PG migration `20260422000000_enum_label_functions`.
//! - **Surrogate ids.** `accounts.id` / `account_balances_current.account_id` /
//!   `transaction_participants.account_id` are the `Int64` cityhash surrogate;
//!   resolved from the G-StrKey via the `accounts` primary key.
//! - **Account transactions span MANY ledger partitions** (an account is
//!   active over time), so the single-partition prune used by the global
//!   `/transactions` list does NOT apply here. Instead the page is driven off
//!   `transaction_participants` (ORDER BY `(account_id, ledger_sequence,
//!   transaction_id)` → an account-scoped primary-key seek), then the ≤ `limit`
//!   transaction rows are fetched by `(ledger_sequence, id) IN (keys)`
//!   (primary-key-prefix prune per ledger, multi-partition-safe) and re-ordered
//!   in Rust. No unpruned `transactions FINAL` join — that would merge the
//!   whole 3.6B-row table (the read_rows-quota trap fixed in the global list).

use std::collections::{BTreeSet, HashMap};

use chrono::{DateTime, Utc};
use clickhouse::Row;
use serde::Deserialize;

use crate::common::ch::{self, millis_to_utc, resolve_accounts};
use crate::common::cursor::{Direction, SortOrder, keyset_sql};
use crate::transactions::dto::TxListCursor;

use super::dto::AccountsListCursor;

// ---------------------------------------------------------------------------
// Internal query-result rows + resolved params (not serialized; the handler
// maps these into the public response DTOs).
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct AccountListRow {
    /// Surrogate id — cursor tiebreak only, never on the wire.
    pub id: i64,
    pub account_id: String,
    pub xlm_balance: Option<String>,
    pub last_seen_ledger: i64,
    pub first_seen_ledger: i64,
    pub home_domain: Option<String>,
}

/// Resolved, validated `GET /v1/accounts` list params handed to `fetch_list`.
pub struct ResolvedListParams {
    pub limit: i64,
    pub cursor: Option<AccountsListCursor>,
    pub with_domain: bool,
}

#[derive(Debug)]
pub struct AccountHeaderRow {
    /// Surrogate id, threaded into balances query — never on wire.
    pub id: i64,
    pub account_id: String,
    pub first_seen_ledger: i64,
    pub last_seen_ledger: i64,
    pub sequence_number: i64,
    pub home_domain: Option<String>,
}

#[derive(Debug)]
pub struct AccountBalanceRow {
    pub asset_type_name: Option<String>,
    pub asset_type: i16,
    pub asset_code: Option<String>,
    pub asset_issuer: Option<String>,
    pub contract_id: Option<String>,
    pub name: Option<String>,
    pub symbol: Option<String>,
    pub balance: String,
    pub decimals: u32,
    pub last_updated_ledger: i64,
    pub sac_deployed: bool,
}

#[derive(Debug)]
pub struct AccountTxRow {
    pub id: i64,
    pub hash: String,
    pub ledger_sequence: i64,
    pub application_order: i16,
    pub source_account: String,
    pub fee_charged: i64,
    pub successful: bool,
    pub operation_count: i16,
    pub has_soroban: bool,
    pub operation_types: Vec<String>,
    pub created_at: DateTime<Utc>,
}

/// `assets.asset_type` SMALLINT → its own domain's label (task 0496).
///
/// The value here comes from the `assets` table, whose domain is
/// [`domain::AssetFamily`] — NOT the XDR `AssetType` this function used to
/// copy its legend from. The two disagree on 3: XDR says `pool_share`, the
/// family says `soroban` — so every Soroban holding (42 975 rows across
/// 38 324 holders, measured on production) rendered as a liquidity-pool
/// share. A renderer may only use the vocabulary of the enum its value came
/// from. `None` for an out-of-range code, unchanged.
fn asset_type_name(asset_type: i16) -> Option<String> {
    domain::AssetFamily::try_from(asset_type)
        .ok()
        .map(|f| f.as_str().to_string())
}

// ---------------------------------------------------------------------------
// List — GET /v1/accounts
// ---------------------------------------------------------------------------

#[derive(Debug, Row, Deserialize)]
struct AccountListChRow {
    id: i64,
    account_id: String,
    last_seen_ledger: i64,
    first_seen_ledger: i64,
    home_domain: Option<String>,
}

/// Step-2 resolve row: `accounts.id` (surrogate) → native (`asset_type=0`) XLM
/// `balance` as text. Keyed on `account_balances_current.account_id`, which is
/// the same surrogate as `accounts.id` (task 0319).
#[derive(Debug, Row, Deserialize)]
struct AccountListBalanceRow {
    account_id: i64,
    balance: String,
}

/// CH equivalent of the PG `queries::fetch_list` (task 0274). Same response
/// shape + same opaque cursor (`AccountsListCursor{last_seen_ledger, id}` —
/// both columns exist on CH `accounts`, so the cursor is datasource-agnostic).
///
/// Divergences from PG:
///
/// - **No `accounts FINAL`** (task 0385): Step 1 pages `accounts_recent`, a
///   refreshable-MV copy already deduped to one row per account and ordered by
///   `(last_seen_ledger, id)`, so it seeks instead of scanning. The native-balance
///   seek (Step 2) is `FINAL` on `balances`/`assets` to collapse re-ingest versions.
/// - **Native balance** is the `asset_type = 0` row of
///   `account_balances_current`, mirroring the PG partial-index join. A
///   `1 AS matched` marker in the subquery + `if(abc.matched = 1, …, NULL)`
///   makes a missing native row decode as `None` (PG `NULL`). We do NOT use
///   `SETTINGS join_use_nulls = 1` — the `api_reader` CH user runs `readonly = 1`
///   (RBAC `read_only` profile), which rejects per-query setting overrides.
/// - **Cursor inlined**, not `.bind()`-ed: the clickhouse 0.15 bound-parameter
///   path returns 0 rows when `None` is bound into a tuple keyset comparison
///   (the documented defect that forced the transactions list to inline). The
///   values are `i64`, so no injection surface.
///
/// **Read cost (task 0385):** Step 1 is a read-in-order SEEK of `accounts_recent`
/// on its `(last_seen_ledger, id)` sort key (~page rows), replacing the old
/// whole-table `accounts FINAL` scan+sort (~24M). A projection on the base RMT
/// was CH-26.3-rejected (task 0353); the refreshable MV is the accepted
/// alternative. Output is ≤(MV refresh interval)-stale on the very newest rows —
/// acceptable for this low-traffic browse list.
pub async fn fetch_list(
    client: &clickhouse::Client,
    params: &ResolvedListParams,
    sort: SortOrder,
    direction: Direction,
) -> Result<Vec<AccountListRow>, clickhouse::error::Error> {
    let (op, order) = keyset_sql(sort, direction);

    // Keyset clause omitted entirely on the first page (no cursor) — the unified
    // CH-list convention — so the clickhouse 0.15 "None into a tuple keyset → 0
    // rows" defect can never fire. The i64 cursor values are inlined (no
    // injection surface).
    let cursor_clause = match &params.cursor {
        Some(c) => format!(
            " AND (a.last_seen_ledger, a.id) {op} ({}, {})",
            c.last_seen_ledger, c.id
        ),
        None => String::new(),
    };
    let domain_filter = if params.with_domain {
        " AND a.home_domain IS NOT NULL"
    } else {
        ""
    };

    // Step 1: page the accounts — NO native-balance join (task 0319, which removed
    // the per-account `account_balances_current FINAL` join that drove the ~2.2s
    // prod TTFB). Source is `accounts_recent` (task 0385): a refreshable-MV copy of
    // `accounts` ORDER BY (last_seen_ledger, id), full recompute + atomic EXCHANGE,
    // so this is a read-in-order SEEK on the list's sort key — not the old
    // `accounts FINAL` + non-PK `last_seen_ledger` whole-dimension scan+sort (a
    // projection was CH-26.3-rejected on the RMT, task 0353). Reads need no FINAL
    // (the MV's EXCHANGE publishes an already-deduped table); freshness = the MV
    // refresh interval.
    let page_sql = format!(
        "SELECT \
            a.id                AS id, \
            a.account_id        AS account_id, \
            a.last_seen_ledger  AS last_seen_ledger, \
            a.first_seen_ledger AS first_seen_ledger, \
            a.home_domain       AS home_domain \
         FROM accounts_recent a \
         WHERE 1{cursor_clause}{domain_filter} \
         ORDER BY a.last_seen_ledger {order}, a.id {order} \
         LIMIT ?"
    );

    let rows = client
        .query(&page_sql)
        .bind(params.limit)
        .fetch_all::<AccountListChRow>()
        .await?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }

    // Step 2: resolve each page account's native (XLM) balance from the unified
    // `balances` table by a PK-prefix key-seek. `balances` is ORDER BY
    // (holder_id, asset_id), so `holder_id IN (…)` seeks the prefix; `FINAL` is
    // bounded to the ≤limit page keys. The IN-list is i64 surrogates (no injection
    // surface), bounded by the page limit.
    let ids = {
        let mut v: Vec<i64> = rows.iter().map(|r| r.id).collect();
        v.sort_unstable();
        v.dedup();
        v.iter().map(i64::to_string).collect::<Vec<_>>().join(",")
    };
    let balances: HashMap<i64, String> = client
        .query(&format!(
            // task 0331 read-cutover: native (XLM) balance now from the unified
            // `balances` table (RAW Int128 = stroops). The frontend scales by
            // `decimals` (7 for native) — same raw-amount contract as the
            // account-detail balances. Resolve native via `assets.asset_type = 0`
            // (the native asset_id is a Rust cityhash, which CH `cityHash64`
            // cannot recompute, so we join rather than hardcode a literal).
            //
            // `closed_at_ledger = 0` for the same reason the detail read uses
            // it (ADR 0055): without it a merged account keeps printing `0 XLM`
            // here while the detail page — which now hides closed rows — shows
            // no XLM at all. The two disagreed before task 0463 in the other
            // direction; the point of the flip is that they agree. A filtered
            // row leaves `xlm_balance` null, which the table renders as a dash.
            "SELECT b.holder_id AS account_id, toString(b.amount) AS balance \
             FROM balances b FINAL \
             INNER JOIN assets a FINAL ON a.id = b.asset_id \
             WHERE b.holder_id IN ({ids}) AND a.asset_type = 0 \
               AND b.closed_at_ledger = 0"
        ))
        .fetch_all::<AccountListBalanceRow>()
        .await?
        .into_iter()
        .map(|r| (r.account_id, r.balance))
        .collect();

    Ok(rows
        .into_iter()
        .map(|r| AccountListRow {
            xlm_balance: balances.get(&r.id).cloned(),
            id: r.id,
            account_id: r.account_id,
            last_seen_ledger: r.last_seen_ledger,
            first_seen_ledger: r.first_seen_ledger,
            home_domain: r.home_domain,
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Detail header — canonical 06 Statement A
// ---------------------------------------------------------------------------

#[derive(Debug, Row, Deserialize)]
struct AccountHeaderChRow {
    id: i64,
    account_id: String,
    first_seen_ledger: i64,
    last_seen_ledger: i64,
    sequence_number: i64,
    home_domain: Option<String>,
}

/// `Ok(None)` → handler returns 404. PK seek on `accounts.account_id` + FINAL
/// (state table; latest version).
pub async fn fetch_account(
    client: &clickhouse::Client,
    account_strkey: &str,
) -> Result<Option<AccountHeaderRow>, clickhouse::error::Error> {
    let row = client
        .query(
            "SELECT \
                a.id, \
                a.account_id, \
                a.first_seen_ledger, \
                a.last_seen_ledger, \
                a.sequence_number, \
                a.home_domain \
             FROM accounts a FINAL \
             WHERE a.account_id = ? \
             LIMIT 1",
        )
        .bind(account_strkey)
        .fetch_optional::<AccountHeaderChRow>()
        .await?;

    Ok(row.map(|r| AccountHeaderRow {
        id: r.id,
        account_id: r.account_id,
        first_seen_ledger: r.first_seen_ledger,
        last_seen_ledger: r.last_seen_ledger,
        sequence_number: r.sequence_number,
        home_domain: r.home_domain,
    }))
}

// ---------------------------------------------------------------------------
// Derived `deleted` status (account_merge) — task 0324
// ---------------------------------------------------------------------------

/// `true` ⟺ the account's ledger entry is GONE — read straight off the
/// lifecycle column on its native holding (ADR 0055), not re-derived from
/// operation history.
///
/// Native XLM lives on the `AccountEntry` itself, so "the account was removed"
/// and "its native holding was closed" are the same fact recorded once. The
/// indexer stamps `closed_at_ledger` when it sees the entry removed, and the
/// checkpoint seed stamped every account that had already gone before our
/// ledger floor — which is what makes this readable now and was not before.
///
/// **Replaces an `operations_appearances` × `transactions` join on the
/// last-seen ledger, which under-detected badly: 22 of 60 sampled merged
/// accounts.** The cause is upstream — a merge operation is not attributed to
/// the account being merged. `GAEGXYY63CYV34TH6HDVZ3L4WCYX7AUTLNOPFCNBR3RCQIB3MVSKLAWP`
/// has its Account Merge in its own `last_seen_ledger`, that ledger holds
/// exactly one type-8 appearance, and none of the 664 appearances there names
/// the account as source or destination; it reaches its own transaction list
/// through `transaction_participants` alone. Deriving a fact from a table that
/// does not carry it cannot be patched into correctness, so this stops trying.
///
/// Chain-verified in both directions via `getLedgerEntries`, 236 accounts, no
/// exceptions:
/// - closed native row → **100 / 100 ABSENT** from the ledger;
/// - open native row → **100 / 100 PRESENT**;
/// - merged and then re-created → **36 / 36 PRESENT**, open row, correctly NOT
///   deleted. The old derivation needed `last_seen_ledger` to handle that case;
///   here it falls out, because a re-create writes a new open row over the
///   tombstone and `FINAL` keeps one row per key (measured: zero accounts hold
///   both an open and a closed native row).
///
/// No native row at all ⇒ `false`. Such an account is not "deleted" — it is one
/// we have only ever seen referenced, never funded, and the caller has already
/// resolved it or returned 404.
pub async fn fetch_deleted_status(
    client: &clickhouse::Client,
    account_surrogate_id: i64,
) -> Result<bool, clickhouse::error::Error> {
    let closed = client
        .query(
            "SELECT closed_at_ledger != 0 \
             FROM balances FINAL \
             WHERE holder_id = ? AND asset_id = ?",
        )
        .bind(account_surrogate_id)
        // The native surrogate is a Rust cityhash CH cannot recompute, so it is
        // BOUND rather than joined for via `assets.asset_type = 0` — same value
        // the writer keys on, one join fewer than the accounts-list read.
        .bind(db_clickhouse::persist::ids::NATIVE_ASSET_ID)
        .fetch_optional::<bool>()
        .await?;

    Ok(closed.unwrap_or(false))
}

// ---------------------------------------------------------------------------
// Detail balances — canonical 06 Statement B
// ---------------------------------------------------------------------------

#[derive(Debug, Row, Deserialize)]
struct AccountBalanceChRow {
    asset_type: i16,
    asset_code: Option<String>,
    issuer_id: i64,
    contract_id: Option<String>,
    name: Option<String>,
    symbol: Option<String>,
    balance: String,
    decimals: u32,
    last_updated_ledger: i64,
    sac_deployed: bool,
}

/// One account's signing configuration as the ledger states it (task 0463,
/// ADR 0055). `None` when the account has no `account_entry_state` row.
#[derive(Debug, clickhouse::Row, serde::Deserialize)]
pub struct AccountEntryStateRow {
    pub signer_keys: Vec<String>,
    pub signer_weights: Vec<u32>,
    pub signer_types: Vec<String>,
    pub master_weight: u8,
    pub threshold_low: u8,
    pub threshold_med: u8,
    pub threshold_high: u8,
    pub last_updated_ledger: i64,
}

/// Read the account's signers + thresholds. First read of
/// `account_entry_state` from the API — the table is written by the indexer
/// (whole-set replacement per observed `AccountEntry`) and seeded for accounts
/// that predate our ledger floor.
///
/// `FINAL` rather than an explicit `argMax`: the table is a
/// `ReplacingMergeTree(last_updated_ledger)` whose parts are not merged in
/// production, so an un-deduplicated read returns several versions of one
/// account. Both forms measured the same on a single-key seek (7 vs 8 ms,
/// 17,465 rows, ~5 MiB), and `FINAL` matches the neighbouring reads in this
/// module. It is also tie-safe here in a way three separate aggregates would
/// not be: it yields ONE stored row, never a mixture of columns from two rows
/// written at the same version.
///
/// **`Ok(None)` means "we have never observed this account's entry state",
/// which is NOT "the account has no extra signers".** 3.7M of 14.6M accounts
/// carry no row (25%), so the two must stay distinguishable all the way to the
/// page — a missing row rendered as "single signature" would be a security
/// claim we cannot support.
pub async fn fetch_entry_state(
    client: &clickhouse::Client,
    account_id: i64,
) -> Result<Option<AccountEntryStateRow>, clickhouse::error::Error> {
    client
        .query(
            "SELECT signer_keys, signer_weights, signer_types, \
                    master_weight, threshold_low, threshold_med, threshold_high, \
                    last_updated_ledger \
             FROM account_entry_state FINAL \
             WHERE account_id = ?",
        )
        .bind(account_id)
        .fetch_optional::<AccountEntryStateRow>()
        .await
}

/// The account-detail balances read, hoisted out of [`fetch_balances`] so the
/// lifecycle predicate is assertable without a live ClickHouse.
///
/// **`closed_at_ledger = 0`, never `amount != 0`** (ADR 0055, task 0463). The
/// old predicate could not tell "holds nothing" from "the trustline is gone":
/// both are `amount = 0`, so hiding zeros hid every zero-balance trustline the
/// account really has, and 78.85% of chain history predates our floor so we
/// cannot recover the difference by re-parsing. The lifecycle column records
/// the removal itself, which is the only thing that distinguishes them.
///
/// Native folds in with no special case: a live account's zero XLM row carries
/// `closed_at_ledger = 0` and shows; a merged account's native tombstone
/// carries a non-zero stamp and does not.
///
/// **Ordering is four keys, and only the first two are load-bearing.** Native
/// is pinned first — it is not a trustline at all (it lives on the
/// `AccountEntry`; every other row is permission granted to an issuer), and its
/// position should never move. Then FUNDED before empty, which is the property
/// the whole change exists for: with zero-balance trustlines now visible, an
/// account can carry thousands of empty rows, and recency alone strands a real
/// holding — measured on a spammed account, a 920bn-unit SGB position at ledger
/// 51,190,951 sits below 3,274 of its empty rows.
///
/// The last two are presentation. `amount DESC` compares RAW amounts across
/// assets with different `decimals` and no prices, so it cannot mean "worth
/// more" and is not claimed to; it just keeps the bigger positions together.
/// Recency then orders the empty rows, where it is the only honest
/// discriminator we have, and `asset_code` makes the whole thing stable so a
/// page boundary never shows the same row twice.
const BALANCES_SQL: &str = "SELECT \
                a.asset_type                  AS asset_type, \
                nullIf(a.asset_code, '')      AS asset_code, \
                a.issuer_id                   AS issuer_id, \
                nullIf(sc.contract_id, '')    AS contract_id, \
                coalesce(nullIf(ae.name, ''), nullIf(m.name, '')) AS name, \
                nullIf(m.symbol, '')          AS symbol, \
                toString(b.amount)            AS balance, \
                coalesce(m.decimals, 7)       AS decimals, \
                b.last_updated_ledger         AS last_updated_ledger, \
                sac.deployed                  AS sac_deployed \
             FROM balances b FINAL \
             INNER JOIN assets a FINAL ON a.id = b.asset_id \
             /* sc FINAL: soroban_contracts is a ReplacingMergeTree with unmerged \
                duplicate ids; without FINAL this join could double balance legs. \
                It was previously dedup'd only incidentally by the adjacent \
                `assets a FINAL` — made explicit here. lore-0420 */ \
             LEFT JOIN soroban_contracts sc FINAL ON sc.id = a.contract_id \
             LEFT JOIN ( \
                 SELECT contract_id, name, symbol, decimals FROM soroban_contract_metadata FINAL \
             ) m ON m.contract_id = sc.contract_id \
             LEFT JOIN ( \
                 SELECT asset_type, asset_code, issuer_id, contract_id, \
                        argMax(name, version) AS name \
                 FROM asset_enrichment \
                 GROUP BY asset_type, asset_code, issuer_id, contract_id \
             ) ae ON ae.asset_type = a.asset_type AND ae.asset_code = a.asset_code \
                 AND ae.issuer_id = a.issuer_id AND ae.contract_id = a.contract_id \
             /* Deployed-SAC facet, the same join `/assets` uses (ADR 0051): a \
                SAC is a PROPERTY of a classic/native asset, never its type, and \
                never a property of the issuer. `asset_sac` is an \
                AggregatingMergeTree, so the state must be aggregated, and the \
                alias cannot be `sac_deployed` — that shadows the column and the \
                HAVING then nests two aggregates (ILLEGAL_AGGREGATION). \
                `HAVING deployed` keeps the joined side to the ~4.3k identities \
                that actually have one, out of ~306k; an unmatched row LEFT \
                JOINs to the column default, which is exactly `false`. \
                \
                RESTRICTED to this holder's own assets before aggregating, the \
                way `/assets` restricts by its page's tuples. Aggregating the \
                whole table first measured 425 MiB and 219 ms per request \
                against 81 MiB / 122 ms without the join; restricted it is \
                86 MiB / 56 ms — the memory, not the latency, is the reason \
                (`read_only` caps a query at 4 GiB, and this runs per page \
                view). Same bound value as the outer WHERE. */ \
             LEFT JOIN ( \
                 SELECT asset_type, asset_code, issuer_id, contract_id, \
                        toBool(max(sac_deployed)) AS deployed \
                 FROM asset_sac \
                 WHERE (asset_type, asset_code, issuer_id, contract_id) IN ( \
                     SELECT asset_type, asset_code, issuer_id, contract_id \
                     FROM assets \
                     WHERE id IN (SELECT asset_id FROM balances WHERE holder_id = ?) \
                 ) \
                 GROUP BY asset_type, asset_code, issuer_id, contract_id \
                 HAVING deployed \
             ) sac ON sac.asset_type = a.asset_type AND sac.asset_code = a.asset_code \
                 AND sac.issuer_id = a.issuer_id AND sac.contract_id = a.contract_id \
             WHERE b.holder_id = ? AND b.closed_at_ledger = 0 \
             ORDER BY a.asset_type = 0 DESC, \
                      b.amount > 0 DESC, \
                      b.amount DESC, \
                      b.last_updated_ledger DESC, \
                      a.asset_code";

/// `account_id` is the surrogate from [`fetch_account`]. Reads the unified
/// `balances` table (task 0331 Option C) by `holder_id` — a leading-PK-prefix
/// seek (`balances` ORDER BY `(holder_id, asset_id)`). Resolves each asset via the
/// `assets.id` surrogate; classic + Soroban (type-3) holdings both appear.
/// `balance` is RAW (`Int128`) — clients scale by `decimals` (classic = 7,
/// Soroban from on-chain `METADATA`).
pub async fn fetch_balances(
    client: &clickhouse::Client,
    account_id: i64,
) -> Result<Vec<AccountBalanceRow>, clickhouse::error::Error> {
    let rows = client
        .query(BALANCES_SQL)
        // Twice: the SAC subquery narrows itself to this holder's assets
        // before aggregating, and the outer read selects them. Same value,
        // bound in the order the two `?` appear.
        .bind(account_id)
        .bind(account_id)
        .fetch_all::<AccountBalanceChRow>()
        .await?;

    // Resolve issuer StrKeys by surrogate id (bloom seek) instead of a
    // whole-`accounts` `LEFT JOIN … ON iss.id = a.issuer_id` (task 0345).
    let accounts = resolve_accounts(client, rows.iter().map(|r| r.issuer_id).collect()).await?;
    Ok(rows
        .into_iter()
        .map(|r| AccountBalanceRow {
            asset_type_name: asset_type_name(r.asset_type),
            asset_type: r.asset_type,
            asset_code: r.asset_code,
            asset_issuer: accounts
                .get(&r.issuer_id)
                .cloned()
                .filter(|s| !s.is_empty()),
            contract_id: r.contract_id,
            name: r.name,
            symbol: r.symbol,
            balance: r.balance,
            decimals: r.decimals,
            last_updated_ledger: r.last_updated_ledger,
            sac_deployed: r.sac_deployed,
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Transactions — canonical 07 (two-step, multi-partition-safe)
// ---------------------------------------------------------------------------

#[derive(Debug, Row, Deserialize)]
struct ParticipantKeyRow {
    ledger_sequence: i64,
    transaction_id: i64,
}

#[derive(Debug, Row, Deserialize)]
struct AccountTxPageChRow {
    id: i64,
    hash: String,
    ledger_sequence: i64,
    application_order: i16,
    source_id: i64,
    fee_charged: i64,
    successful: bool,
    operation_count: i16,
    has_soroban: bool,
    created_at: i64,
}

/// `account_id` is the surrogate from [`fetch_account`]; caller passes
/// `limit + 1` (the peek row drives forward-continuation detection). The CH
/// cursor keys on `(ledger_sequence, transaction_id)` — a `Pg`-variant cursor
/// never reaches here (`list_account_transactions` rejects a cross-datasource
/// cursor before dispatch), so the `_` arm only ever means "first page".
pub async fn fetch_transactions(
    client: &clickhouse::Client,
    account_id: i64,
    limit: i64,
    cursor: Option<&TxListCursor>,
    sort: SortOrder,
    direction: Direction,
) -> Result<Vec<AccountTxRow>, clickhouse::error::Error> {
    let (cursor_ledger, cursor_tiebreak): (Option<i64>, Option<i64>) = match cursor {
        Some(TxListCursor::Ch {
            ledger_sequence,
            tiebreak,
        }) => (Some(*ledger_sequence), Some(*tiebreak)),
        _ => (None, None),
    };
    let (op, order) = keyset_sql(sort, direction);

    // Inline the cursor bounds rather than `.bind()`-ing them: the clickhouse
    // 0.15 bound-parameter path returns an empty result when `None` is bound
    // into a tuple keyset comparison — the same defect that forced the
    // transactions B/C statements to inline (a bound first page silently
    // returned 0 rows). Values are i64 / None→NULL, so no injection surface.
    let cl = cursor_ledger.map_or_else(|| "NULL".to_string(), |v| v.to_string());
    let ct = cursor_tiebreak.map_or_else(|| "NULL".to_string(), |v| v.to_string());

    // Step 1: account-scoped driver seek. `account_id` is the leading PK of
    // `transaction_participants`. FINAL is dropped: on a hot account it merges
    // the account's rows across every part — measured 1.16B vs 327M rows read
    // (3.5x), and one such FINAL page is ~11% of the hourly read_rows quota.
    // `LIMIT 1 BY (ledger_sequence, transaction_id)` collapses any rare
    // re-ingest duplicate, so the page still yields `limit` distinct keys
    // (box-verified: 21/21 distinct on the hottest account).
    let driver_sql = format!(
        "SELECT tp.ledger_sequence AS ledger_sequence, tp.transaction_id AS transaction_id \
         FROM transaction_participants tp \
         WHERE tp.account_id = ? \
           AND tp.ledger_sequence <= (SELECT max(sequence) FROM ledgers) \
           AND ({cl} IS NULL OR (tp.ledger_sequence, tp.transaction_id) {op} ({cl}, {ct})) \
         ORDER BY tp.ledger_sequence {order}, tp.transaction_id {order} \
         LIMIT 1 BY tp.ledger_sequence, tp.transaction_id \
         LIMIT ?"
    );
    let key_rows = client
        .query(&driver_sql)
        .bind(account_id)
        .bind(limit)
        .fetch_all::<ParticipantKeyRow>()
        .await?;

    if key_rows.is_empty() {
        return Ok(Vec::new());
    }

    let keys: Vec<(i64, i64)> = key_rows
        .iter()
        .map(|r| (r.ledger_sequence, r.transaction_id))
        .collect();

    // Step 2: fetch the ≤limit transaction rows by `(ledger_sequence, id) IN
    // (keys)` (primary-key-prefix prune per ledger; spans partitions safely),
    // concurrently with the operation_types aggregate for the same keys.
    // Keys are `i64`, inlined directly — no injection surface.
    let in_tuples = keys
        .iter()
        .map(|(ledger, tx)| format!("({ledger},{tx})"))
        .collect::<Vec<_>>()
        .join(",");
    let partitions = keys
        .iter()
        .map(|(ledger, _)| ledger / 500_000)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let page_sql = format!(
        "SELECT \
            t.id AS id, \
            lower(hex(t.hash)) AS hash, \
            t.ledger_sequence, \
            t.application_order, \
            t.source_id AS source_id, \
            t.fee_charged, \
            t.successful, \
            t.operation_count, \
            t.has_soroban, \
            l.closed_at AS created_at \
         FROM transactions t \
         INNER JOIN ledgers l ON l.sequence = t.ledger_sequence \
         WHERE (t.ledger_sequence, t.id) IN ({in_tuples}) \
           AND intDiv(t.ledger_sequence, 500000) IN ({partitions})"
    );
    let (page_rows, aggregates) = tokio::join!(
        client.query(&page_sql).fetch_all::<AccountTxPageChRow>(),
        ch::fetch_tx_list_aggregates(client, &keys),
    );
    let page_rows = page_rows?;
    let aggregates = aggregates?;

    // Resolve source StrKeys by surrogate id (bloom seek) instead of a
    // whole-`accounts` `LEFT JOIN … ON src.id = t.source_id` (task 0345).
    let accounts =
        resolve_accounts(client, page_rows.iter().map(|r| r.source_id).collect()).await?;

    // Step 3: index page rows by id (a re-ingested tx collapses — values are
    // immutable), then emit in the driver's keyset order, merging
    // operation_types (the only aggregate the item carries).
    let mut by_id: HashMap<i64, AccountTxPageChRow> = HashMap::with_capacity(page_rows.len());
    for row in page_rows {
        by_id.insert(row.id, row);
    }

    let mut out = Vec::with_capacity(keys.len());
    for (_, tx_id) in &keys {
        let Some(row) = by_id.remove(tx_id) else {
            continue;
        };
        let agg = aggregates.get(tx_id);
        let operation_types = agg.map(|a| a.operation_types.clone()).unwrap_or_default();
        out.push(AccountTxRow {
            id: row.id,
            hash: row.hash,
            ledger_sequence: row.ledger_sequence,
            application_order: row.application_order,
            source_account: accounts
                .get(&row.source_id)
                .cloned()
                .filter(|s| !s.is_empty())
                .unwrap_or_default(),
            fee_charged: row.fee_charged,
            successful: row.successful,
            operation_count: row.operation_count,
            has_soroban: row.has_soroban,
            operation_types,
            created_at: millis_to_utc(row.created_at),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The account-detail read must select on the LIFECYCLE column, never on
    /// the amount. `amount != 0` cannot tell "holds nothing" from "the
    /// trustline is gone" — it was the defect behind issue #377, and it is a
    /// one-word regression away, so the predicate is pinned here rather than
    /// left to review. Asserted against the SQL itself, no ClickHouse needed.
    #[test]
    fn balances_are_selected_by_lifecycle_not_by_amount() {
        assert!(
            BALANCES_SQL.contains("b.closed_at_ledger = 0"),
            "the balances read must filter on the lifecycle column"
        );
        assert!(
            !BALANCES_SQL.contains("b.amount != 0"),
            "`amount != 0` hides every zero-balance trustline the account holds"
        );
        // A zero-amount row that is still open has to survive the predicate,
        // which is only true if `amount` is absent from the WHERE clause
        // entirely — a combined `amount != 0 OR ...` would pass the check above.
        let where_clause = BALANCES_SQL
            .split("WHERE")
            .nth(1)
            .expect("the read has a WHERE clause");
        assert!(
            !where_clause.contains("amount"),
            "no amount predicate belongs in this WHERE clause: {where_clause}"
        );
    }

    /// The old version of this test pinned the XDR legend onto family values
    /// and thereby froze bug 0496 in place: it asserted 3 = `pool_share`, so
    /// every Soroban holding rendered as a liquidity-pool share and the test
    /// was green. A parity test is only as good as the enum it picks.
    #[test]
    fn asset_type_name_speaks_the_family_vocabulary() {
        assert_eq!(asset_type_name(0).as_deref(), Some("native"));
        assert_eq!(asset_type_name(1).as_deref(), Some("classic_credit"));
        assert_eq!(
            asset_type_name(3).as_deref(),
            Some("soroban"),
            "3 is AssetFamily::Soroban here, never the XDR pool_share"
        );
        assert_eq!(asset_type_name(2), None, "2 (sac) is retired — ADR 0051");
        assert_eq!(asset_type_name(99), None);
    }
}
