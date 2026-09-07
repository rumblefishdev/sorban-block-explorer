//! ClickHouse queries backing `GET /v1/search` (task 0318).
//!
//! Returns `Vec<(String, SearchHit)>` with per-bucket `LIMIT` semantics,
//! so the handler stays backend-agnostic after the fetch. This is the sole
//! read path — PG was retired in prod (ADR 0047) and removed (task 0244).
//!
//! # Design — classification-gated, concurrent, per-bucket
//!
//! The PG implementation is one UNION of six WHERE-gated CTEs; CH evaluates
//! every branch even when its input is NULL, so a hash query would still scan
//! `assets` / `nfts` / `soroban_contracts` for a substring that provably (or
//! pathologically) cannot match. Instead this module fires **only the buckets
//! the classifier proves can match**, concurrently (`tokio::try_join!`):
//!
//! | mode (classifier)        | buckets fired                                |
//! |--------------------------|----------------------------------------------|
//! | `hash_bytes` (hex / L)   | `transaction`, `pool`                         |
//! | `strkey_prefix` (G… / C…)| `account`, `contract` (prefix), `asset`, `nft`|
//! | plain text               | `contract` (name), `asset`, `nft`             |
//!
//! This is the minimal-rows-scanned shape (the common hash lookup touches two
//! point-seeks and nothing else) and stays faithful to the PG result set for
//! every realistic input. The one intentional divergence: in `hash_bytes` mode
//! we skip the small-table substring buckets — `asset_code` (≤12 chars) can
//! never contain a 64-hex / 56-char needle (provably empty), and a contract /
//! NFT *named* after a full transaction hash is not a real search intent.
//!
//! # CH-vs-PG / CH-vs-canonical-SQL divergences (all verified against
//! `crates/db-clickhouse/schema/init.sql` and the live CH read modules)
//!
//! - **Transaction lookup** reads `transaction_hash_index` (ORDER BY `hash`,
//!   PK point-seek), mirroring [`crate::transactions::queries`]. The
//!   canonical `22_get_search.sql` proposes the `transaction_hash_dict`
//!   Dictionary hot path; that is a CH-only optimisation layerable later
//!   without changing this contract. `successful` + `last_activity_at` are
//!   resolved (PG parity) via a partition-pruned `transactions` seek + a
//!   `ledgers` PK join — both single-row, so the cost is two point-seeks.
//! - **NFT name** lives in `nft_enrichment`, NOT `nfts.name` (vestigial NULL on
//!   CH — the live indexer rewrites whole `nfts` rows on every ownership change
//!   with metadata NULL; task 0231). The canonical SQL's `nfts.name` predicate
//!   would silently match nothing. We collapse the enrichment with
//!   `argMax(_, version)` (never `FINAL`) exactly like [`crate::nfts::queries`].
//! - **Contract name** lives in `soroban_contract_metadata` (on-chain METADATA
//!   struct), NOT the dead `soroban_contracts.name` (no writer since task 0297).
//!   Contract free-text therefore matches Soroban-native tokens by their
//!   on-chain name; SACs are excluded from that table by design.
//! - **Asset issuer / contract StrKey** are resolved without a full-table hash
//!   join: the issuer surrogate → G-StrKey is a bloom-pruned
//!   `accounts WHERE id IN (page ids)` key-seek (`idx_acc_id`), never
//!   `LEFT JOIN accounts` (the ~23M-row hash-side build that OOMs — CH Code 241,
//!   the 0317 trap the canonical asset CTE would have hit). `soroban_contracts`
//!   (smaller) is joined in-statement, same as the live `/assets` list.
//! - **`nullIf(...)`** maps a sentinel / JOIN miss to `None`. We do NOT use
//!   `SETTINGS join_use_nulls = 1` — `api_reader` runs `readonly = 1` (RBAC
//!   `read_only`) and rejects per-query setting overrides.
//! - **Positional `clickhouse::Row` decode:** every `SELECT` column order MUST
//!   match its Row struct field order; a reorder silently decodes into the
//!   wrong field.

use std::collections::{BTreeSet, HashMap};

use clickhouse::Row;
use serde::Deserialize;

use crate::common::ch::millis_to_utc;
use crate::common::pool_asset_codes::{asset_codes_predicate, normalize_asset_codes};
use crate::common::strkey::pool_id_hex_to_strkey;

use super::classifier::Classified;
use super::dto::{EntityType, SearchHit};

/// Which entity buckets the `?type=` filter admits. Parsed from the query
/// string (backend-agnostic); passed into `fetch_search` to gate the
/// per-entity CTE branches.
#[derive(Debug, Clone, Copy)]
pub struct IncludeFlags {
    pub transaction: bool,
    pub contract: bool,
    pub asset: bool,
    pub account: bool,
    pub nft: bool,
    pub pool: bool,
}

impl IncludeFlags {
    pub fn all() -> Self {
        Self {
            transaction: true,
            contract: true,
            asset: true,
            account: true,
            nft: true,
            pool: true,
        }
    }

    pub fn none() -> Self {
        Self {
            transaction: false,
            contract: false,
            asset: false,
            account: false,
            nft: false,
            pool: false,
        }
    }

    pub fn enable(&mut self, t: EntityType) {
        match t {
            EntityType::Transaction => self.transaction = true,
            EntityType::Contract => self.contract = true,
            EntityType::Asset => self.asset = true,
            EntityType::Account => self.account = true,
            EntityType::Nft => self.nft = true,
            EntityType::Pool => self.pool = true,
        }
    }
}

/// Mainnet ledger-partition width (`PARTITION BY intDiv(ledger_sequence,
/// 500000)` on `transactions`). Used to prune the `transactions` seek to the
/// single partition the `transaction_hash_index` lookup resolved.
const LEDGER_PARTITION_SIZE: i64 = 500_000;

/// `asset_type` SMALLINT → canonical label, matching the PG `asset_family_name`
/// function (same mapping as [`crate::assets::queries`], NOT the
/// `AssetType`-XDR `asset_type_name` used by accounts). `None` for an
/// out-of-range code (the PG `CASE` returns NULL with no `ELSE`).
fn asset_family_name(asset_type: i16) -> Option<String> {
    match asset_type {
        0 => Some("native"),
        1 => Some("classic_credit"),
        // 2 (`sac`) retired — ADR 0051 (mirrors `assets::queries::asset_type_name`).
        3 => Some("soroban"),
        _ => None,
    }
    .map(str::to_string)
}

/// Canonical `/assets/:id` routing token, mirroring `canonical_id` in
/// `assets/handlers.rs` and the PG `asset_hits` CTE: contract StrKey if present,
/// else `CODE-ISSUER` (both parts resolved), else `native` (gated on
/// `asset_type = 0`). A malformed row (code present, issuer unresolved) yields
/// `None` — an honest no-route, never a mis-route to the native XLM page.
fn asset_route_token(
    contract_strkey: Option<&str>,
    asset_code: Option<&str>,
    issuer: Option<&str>,
    asset_type: i16,
) -> Option<String> {
    contract_strkey
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| match (asset_code, issuer) {
            (Some(code), Some(iss)) if !code.is_empty() && !iss.is_empty() => {
                Some(format!("{code}-{iss}"))
            }
            _ => None,
        })
        .or_else(|| (asset_type == 0).then(|| "native".to_string()))
}

// ---------------------------------------------------------------------------
// Orchestrator
// ---------------------------------------------------------------------------

/// Runs the broad search. Fires the buckets the
/// classifier proves can match (see module docs), concurrently, and returns the
/// rows partitioned by `entity_type` for the handler to group. The returned
/// tuple's `String` is the entity-type literal (the handler ignores it and uses
/// the typed `SearchHit::entity_type`); kept for shape-parity with the PG path.
pub async fn fetch_search(
    client: &clickhouse::Client,
    q: &str,
    classified: &Classified,
    include: &IncludeFlags,
    per_group_limit: i32,
) -> Result<Vec<(String, SearchHit)>, clickhouse::error::Error> {
    let (tx, contract, asset, account, nft, pool) = tokio::try_join!(
        search_transactions(client, classified, include),
        search_contracts(client, q, classified, include, per_group_limit),
        search_assets(client, q, classified, include, per_group_limit),
        search_accounts(client, classified, include, per_group_limit),
        search_nfts(client, q, classified, include, per_group_limit),
        search_pools(client, q, classified, include, per_group_limit),
    )?;

    // Canonical union order (irrelevant to grouping — the handler buckets by
    // `entity_type` — but keeps the flat list tidy / diff-stable).
    let mut out = Vec::with_capacity(
        tx.len() + contract.len() + asset.len() + account.len() + nft.len() + pool.len(),
    );
    out.extend(tx);
    out.extend(contract);
    out.extend(asset);
    out.extend(account);
    out.extend(nft);
    out.extend(pool);
    Ok(out)
}

// ---------------------------------------------------------------------------
// Transactions — exact hash → transaction_hash_index PK seek
// ---------------------------------------------------------------------------

#[derive(Debug, Row, Deserialize)]
struct LedgerSeqRow {
    ledger_sequence: i64,
}

#[derive(Debug, Row, Deserialize)]
struct TxMetaRow {
    successful: bool,
    /// `ledgers.closed_at` (`DateTime64(3)`) decoded as raw i64 millis.
    created_at_ms: i64,
}

/// Fires only for a hash-shaped query. Step 1 resolves `hash → ledger_sequence`
/// off `transaction_hash_index` (ORDER BY `hash`; immutable mapping, no FINAL).
/// Step 2 reads `successful` + the ledger `closed_at` via a single-partition,
/// single-row seek on `transactions` (`ledger_sequence` leading PK + the
/// `idx_tx_hash_bloom` filter) joined to `ledgers` — the PG `tx_hits`
/// enrichment, at the cost of two point-seeks.
async fn search_transactions(
    client: &clickhouse::Client,
    classified: &Classified,
    include: &IncludeFlags,
) -> Result<Vec<(String, SearchHit)>, clickhouse::error::Error> {
    if !include.transaction {
        return Ok(Vec::new());
    }
    let Some(bytes) = classified.hash_bytes.as_deref() else {
        return Ok(Vec::new());
    };
    let hash_hex = hex::encode(bytes);

    let ledger = client
        .query(
            "SELECT ledger_sequence FROM transaction_hash_index \
             WHERE hash = unhex(?) LIMIT 1",
        )
        .bind(&hash_hex)
        .fetch_optional::<LedgerSeqRow>()
        .await?
        .map(|r| r.ledger_sequence);
    let Some(ledger) = ledger else {
        return Ok(Vec::new());
    };

    // `ledger` is from our own index seek (i64), inlined for partition pruning —
    // no injection surface. `hash` is bound. `successful` is immutable across
    // re-ingest, so `LIMIT 1` (no FINAL) is correct. `closed_at` is resolved via
    // a BOUNDED `ledgers WHERE sequence = {ledger}` sub-select (PK point seek, ~1
    // granule) — NOT a plain `INNER JOIN ledgers`, which builds its hash side
    // from the whole ~3.6M-row `ledgers` table on prod (measured: a scalar
    // correlated form also de-optimised to a full scan). INNER (not LEFT): under
    // the `api_reader` `join_use_nulls = 0` RBAC a LEFT-JOIN miss yields the
    // column DEFAULT on a NON-nullable column (not NULL), so LEFT buys no real
    // "missing ledger ⇒ None" semantics — it would only mask a 0-epoch. A real
    // indexed tx always has its ledger row, so INNER is correct and simpler.
    let partition = ledger / LEDGER_PARTITION_SIZE;
    let sql = format!(
        "SELECT t.successful AS successful, l.closed_at AS created_at_ms \
         FROM transactions t \
         INNER JOIN (SELECT sequence, closed_at FROM ledgers WHERE sequence = {ledger}) l \
                 ON l.sequence = t.ledger_sequence \
         WHERE t.ledger_sequence = {ledger} \
           AND intDiv(t.ledger_sequence, {LEDGER_PARTITION_SIZE}) = {partition} \
           AND t.hash = unhex(?) \
         ORDER BY t.application_order \
         LIMIT 1"
    );
    let Some(meta) = client
        .query(&sql)
        .bind(&hash_hex)
        .fetch_optional::<TxMetaRow>()
        .await?
    else {
        // Index row present but the transaction row is missing — treat as no
        // hit rather than emit a half-populated redirect target.
        return Ok(Vec::new());
    };

    Ok(vec![(
        "transaction".to_string(),
        SearchHit {
            entity_type: EntityType::Transaction,
            identifier: hash_hex,
            label: String::new(),
            route_token: None,
            successful: Some(meta.successful),
            last_activity_at: Some(millis_to_utc(meta.created_at_ms)),
            contract_id: None,
            token_id: None,
        },
    )])
}

// ---------------------------------------------------------------------------
// Pools — exact pool_id (from a 64-hex or full L-StrKey) → PK point seek
// ---------------------------------------------------------------------------

#[derive(Debug, Row, Deserialize)]
struct PoolRow {
    pool_hex: String,
    label: String,
}

/// The pool's display name, identical in both arms — a hit found by id and the
/// same pool found by code must not be labelled differently.
///
/// Native is detected by `asset_type = 0`, the SAME signal
/// `common::pool_asset_codes` matches on. This used to key off the empty stored
/// code instead, which gave the label and the predicate two different notions
/// of "is this XLM" over one row. Two native conventions have already produced
/// real bugs here — an empty code slips through filters that a type check
/// catches — so the second one is gone rather than commented.
///
/// Byte-identical output, verified before the change: over all 75 218 rows,
/// zero disagree on either leg. Nothing on the wire moves.
const POOL_LABEL_SQL: &str = "concat( \
     if(asset_a_type = 0, 'XLM', toString(asset_a_code)), ' / ', \
     if(asset_b_type = 0, 'XLM', toString(asset_b_code)) \
 ) AS label";

/// Two shapes, one entity.
///
/// A hash-shaped query is a point seek on the primary key and stays exactly as
/// it was. Anything else is treated as an asset code and matched with the SAME
/// rule the pools list uses (`common::pool_asset_codes`, task 0470) — before
/// that, a non-hash query matched no pool at all, so `KALE` reported zero here
/// while the pools page listed 58, which read as the 0440 fix not working.
async fn search_pools(
    client: &clickhouse::Client,
    q: &str,
    classified: &Classified,
    include: &IncludeFlags,
    per_group_limit: i32,
) -> Result<Vec<(String, SearchHit)>, clickhouse::error::Error> {
    if !include.pool {
        return Ok(Vec::new());
    }
    match classified.hash_bytes.as_deref() {
        Some(bytes) => search_pool_by_id(client, bytes).await,
        None => search_pools_by_asset_code(client, q, per_group_limit).await,
    }
}

/// Longest Stellar asset code (`alphanum12`); `alphanum4` is the 1–4 case.
/// Anything longer cannot be a code, so it cannot match a pool.
const MAX_ASSET_CODE_LEN: usize = 12;

/// Pools whose displayed asset codes match the query. Full scan of
/// `liquidity_pools` — 52 472 pools / 73 880 rows, measured at 47 ms and
/// 3.3 MiB on production, on the arm that previously did nothing for this
/// input.
///
/// `argMax(… , last_updated_ledger)` per `pool_id` collapses re-ingest
/// versions on read: the table carries multiple rows per pool (measured: 73 880
/// rows for 52 472 pools) and matching without the grouping would return the
/// same pool several times.
///
/// The pools LIST dedups the same table with `FINAL` instead. That is right
/// there — it reads one page through the primary key — and wrong here, where
/// the predicate cannot be pushed down and `FINAL` would merge the whole table
/// to answer a search box (task 0420 measured a 19x read amplification from
/// exactly that shape). Same goal, different access pattern, deliberately
/// different mechanism.
async fn search_pools_by_asset_code(
    client: &clickhouse::Client,
    q: &str,
    per_group_limit: i32,
) -> Result<Vec<(String, SearchHit)>, clickhouse::error::Error> {
    let codes = normalize_asset_codes(Some(q.to_string()));
    // A Stellar asset code is 1–12 characters (alphanum4 / alphanum12), so a
    // longer needle cannot match any pool. Without this gate every
    // account- or contract-shaped query — a 56-character StrKey — paid the
    // scan below for a guaranteed-empty result, and `/v1/search` runs its six
    // buckets in parallel, so that cost lands on the tail of every such search.
    if codes.iter().any(|c| c.chars().count() > MAX_ASSET_CODE_LEN) {
        return Ok(Vec::new());
    }
    let Some((clause, binds)) = asset_codes_predicate(&codes) else {
        return Ok(Vec::new());
    };
    let sql = format!(
        "SELECT pool_hex, {POOL_LABEL_SQL} \
         FROM ( \
            SELECT \
                lower(hex(pool_id)) AS pool_hex, \
                argMax(asset_a_type, last_updated_ledger) AS asset_a_type, \
                argMax(asset_a_code, last_updated_ledger) AS asset_a_code, \
                argMax(asset_b_type, last_updated_ledger) AS asset_b_type, \
                argMax(asset_b_code, last_updated_ledger) AS asset_b_code, \
                max(last_updated_ledger) AS newest \
            FROM liquidity_pools \
            GROUP BY pool_id \
         ) AS lp \
         WHERE {clause} \
         ORDER BY newest DESC \
         LIMIT ?"
    );

    let mut query = client.query(&sql);
    for bind in &binds {
        query = query.bind(bind);
    }
    let rows = query.bind(per_group_limit).fetch_all::<PoolRow>().await?;

    Ok(rows.into_iter().map(|p| pool_hit(&p)).collect())
}

/// Fires only for a hash-shaped query. `pool_id` is the full ORDER BY key, so
/// `pool_id = unhex(?)` is a granule-pruned point seek. Codes are version-stable
/// (composition is fixed; only reserves move `last_updated_ledger`), so
/// `ORDER BY last_updated_ledger DESC LIMIT 1` collapses re-ingest versions to
/// one row without a FINAL merge.
async fn search_pool_by_id(
    client: &clickhouse::Client,
    bytes: &[u8],
) -> Result<Vec<(String, SearchHit)>, clickhouse::error::Error> {
    let hash_hex = hex::encode(bytes);

    let row = client
        .query(&format!(
            "SELECT lower(hex(pool_id)) AS pool_hex, {POOL_LABEL_SQL} \
             FROM liquidity_pools \
             WHERE pool_id = unhex(?) \
             ORDER BY last_updated_ledger DESC \
             LIMIT 1"
        ))
        .bind(&hash_hex)
        .fetch_optional::<PoolRow>()
        .await?;
    let Some(p) = row else {
        return Ok(Vec::new());
    };

    Ok(vec![pool_hit(&p)])
}

/// Shared by both pool arms so an id hit and a code hit cannot describe the
/// same pool differently.
fn pool_hit(p: &PoolRow) -> (String, SearchHit) {
    (
        "pool".to_string(),
        SearchHit {
            entity_type: EntityType::Pool,
            // Wire identifier is the canonical `L…` strkey (ADR 0008 / task
            // 0264); the column projects raw hex — convert at the boundary,
            // same as the PG row-mapper.
            identifier: pool_id_hex_to_strkey(&p.pool_hex),
            label: p.label.clone(),
            route_token: None,
            successful: None,
            last_activity_at: None,
            contract_id: None,
            token_id: None,
        },
    )
}

// ---------------------------------------------------------------------------
// Accounts — G-StrKey prefix → PK range scan
// ---------------------------------------------------------------------------

#[derive(Debug, Row, Deserialize)]
struct AccountRow {
    account_id: String,
    label: String,
}

/// Fires only when the classifier produced a StrKey prefix. `account_id` is the
/// `accounts` ORDER BY key, so `startsWith(account_id, prefix) ORDER BY
/// account_id LIMIT` is an **early-terminating** primary-key range read — it
/// stops after a few granules regardless of how broad the prefix is.
///
/// **No `FINAL`** — a `FINAL` over the prefix range must merge the WHOLE range
/// before `LIMIT` applies, so its cost scales with the prefix breadth (measured:
/// `GA` read 98k/246k rows locally → ~9M on the 23M-row prod table). The plain
/// read instead reads a constant ~4 granules (measured: 33k for the broadest
/// `G` prefix). Re-ingest duplicates of one `account_id` are contiguous in sort
/// order and collapsed in Rust (keep-first); `home_domain` is version-stable
/// enough for a dropdown label, and post-merge the table is ~1 row per account.
/// A `C…` prefix matches no account (cheap empty range), mirroring the PG gate.
async fn search_accounts(
    client: &clickhouse::Client,
    classified: &Classified,
    include: &IncludeFlags,
    per_group_limit: i32,
) -> Result<Vec<(String, SearchHit)>, clickhouse::error::Error> {
    if !include.account {
        return Ok(Vec::new());
    }
    let Some(prefix) = classified.strkey_prefix.as_deref() else {
        return Ok(Vec::new());
    };

    // `per_group_limit` is a handler-validated 1..=50 (no injection surface).
    let sql = format!(
        "SELECT account_id AS account_id, ifNull(home_domain, '') AS label \
         FROM accounts \
         WHERE startsWith(account_id, ?) \
         ORDER BY account_id \
         LIMIT {per_group_limit}"
    );
    let rows = client
        .query(&sql)
        .bind(prefix)
        .fetch_all::<AccountRow>()
        .await?;

    // Collapse adjacent re-ingest duplicates of the same `account_id` (rows are
    // `account_id`-ordered, so dupes are contiguous).
    let mut out = Vec::with_capacity(rows.len());
    let mut last_id: Option<String> = None;
    for r in rows {
        if last_id.as_deref() == Some(r.account_id.as_str()) {
            continue;
        }
        last_id = Some(r.account_id.clone());
        out.push((
            "account".to_string(),
            SearchHit {
                entity_type: EntityType::Account,
                identifier: r.account_id,
                label: r.label,
                route_token: None,
                successful: None,
                last_activity_at: None,
                contract_id: None,
                token_id: None,
            },
        ));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Contracts — C-StrKey prefix (PK range) OR free-text name (metadata)
// ---------------------------------------------------------------------------

#[derive(Debug, Row, Deserialize)]
struct ContractIdRow {
    contract_id: String,
}

#[derive(Debug, Row, Deserialize)]
struct ContractNameRow {
    contract_id: String,
    name: String,
}

/// Two sub-modes, mirroring the PG `contract_hits` CTE:
/// * StrKey prefix set → `startsWith(contract_id, prefix) ORDER BY contract_id
///   LIMIT` PK range on `soroban_contracts`. **No `FINAL`** (same reasoning as
///   `search_accounts`: a FINAL merge over the prefix range scales with breadth;
///   `contract_id` is the immutable identity so the read needs no version
///   collapse) — duplicates are collapsed in Rust. The display name is then
///   resolved for the ≤`limit` matched ids via a BOUNDED `soroban_contract_
///   metadata WHERE contract_id IN (...)` PK seek (the live name source —
///   `soroban_contracts.name` is dead), NOT a per-query aggregation of the whole
///   metadata table.
/// * plain text → substring over `soroban_contract_metadata.name` (the
///   `$3 IS NULL` branch). Matches Soroban-native tokens by their on-chain
///   name; SACs are excluded from that table by design.
///
/// In `hash_bytes` mode (neither sub-mode applies) the bucket is skipped — a
/// contract named after a full transaction hash is not a real search intent.
async fn search_contracts(
    client: &clickhouse::Client,
    q: &str,
    classified: &Classified,
    include: &IncludeFlags,
    per_group_limit: i32,
) -> Result<Vec<(String, SearchHit)>, clickhouse::error::Error> {
    if !include.contract {
        return Ok(Vec::new());
    }

    // Resolved (contract_id, label) pairs, in match order.
    let pairs: Vec<(String, String)> = if let Some(prefix) = classified.strkey_prefix.as_deref() {
        // Phase 1: early-terminating PK-range scan for the matching contract ids.
        let sql = format!(
            "SELECT contract_id AS contract_id \
             FROM soroban_contracts \
             WHERE startsWith(contract_id, ?) \
             ORDER BY contract_id \
             LIMIT {per_group_limit}"
        );
        let id_rows = client
            .query(&sql)
            .bind(prefix)
            .fetch_all::<ContractIdRow>()
            .await?;

        // Collapse adjacent re-ingest duplicates (rows are contract_id-ordered).
        let mut ids: Vec<String> = Vec::with_capacity(id_rows.len());
        for r in id_rows {
            if ids.last().map(String::as_str) != Some(r.contract_id.as_str()) {
                ids.push(r.contract_id);
            }
        }
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        // Phase 2: bounded name resolution — `contract_id` is the metadata PK, so
        // `IN (...)` is a key-seek over only the matched ids. The user-derived
        // StrKeys are `.bind()`-ed as an array.
        let placeholders = vec!["?"; ids.len()].join(",");
        // `ifNull(argMax(...), '')`: `soroban_contract_metadata.name` is
        // `Nullable(String)`, so `argMax` projects a Nullable column. The wire
        // type MUST be non-nullable to decode into `ContractNameRow.name: String`
        // — without `ifNull` a contract whose latest metadata name is NULL makes
        // the RowBinary decoder mismatch (500). Empty string ⇒ "no name".
        let name_sql = format!(
            "SELECT contract_id AS contract_id, ifNull(argMax(name, version), '') AS name \
             FROM soroban_contract_metadata \
             WHERE contract_id IN ({placeholders}) \
             GROUP BY contract_id"
        );
        let mut name_q = client.query(&name_sql);
        for id in &ids {
            name_q = name_q.bind(id);
        }
        let names: HashMap<String, String> = name_q
            .fetch_all::<ContractNameRow>()
            .await?
            .into_iter()
            .map(|r| (r.contract_id, r.name))
            .collect();

        ids.into_iter()
            .map(|id| {
                let label = names.get(&id).cloned().unwrap_or_default();
                (id, label)
            })
            .collect()
    } else if classified.hash_bytes.is_none() {
        // Plain-text name search over the on-chain metadata. Subquery (not
        // HAVING) so the `argMax` name is computed once and filtered. The outer
        // `ifNull(..., '')` keeps the projected column non-nullable so it decodes
        // into `ContractNameRow.name: String` (the WHERE filter alone does not
        // change the column's Nullable wire type).
        let sql = format!(
            "SELECT contract_id AS contract_id, ifNull(name, '') AS name FROM ( \
                 SELECT contract_id, argMax(name, version) AS name \
                 FROM soroban_contract_metadata GROUP BY contract_id \
             ) \
             WHERE positionCaseInsensitive(ifNull(name, ''), ?) > 0 \
             LIMIT {per_group_limit}"
        );
        client
            .query(&sql)
            .bind(q)
            .fetch_all::<ContractNameRow>()
            .await?
            .into_iter()
            .map(|r| (r.contract_id, r.name))
            .collect()
    } else {
        return Ok(Vec::new());
    };

    Ok(pairs
        .into_iter()
        .map(|(contract_id, label)| {
            (
                "contract".to_string(),
                SearchHit {
                    entity_type: EntityType::Contract,
                    identifier: contract_id,
                    label,
                    route_token: None,
                    successful: None,
                    last_activity_at: None,
                    contract_id: None,
                    token_id: None,
                },
            )
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Assets — asset_code substring (+ native special-case), two-step resolve
// ---------------------------------------------------------------------------

#[derive(Debug, Row, Deserialize)]
struct AssetPhase1Row {
    asset_type: i16,
    asset_code: Option<String>,
    /// `soroban_contracts.contract_id` (C-StrKey) via the in-statement join,
    /// `nullIf`-collapsed on a miss.
    contract_strkey: Option<String>,
    /// Surrogate `accounts.id`; `0` = no issuer (native / soroban-native).
    issuer_id: i64,
}

#[derive(Debug, Row, Deserialize)]
struct IssuerRow {
    id: i64,
    account_id: String,
}

/// Fires whenever the query is NOT hash-shaped (a 64-hex / 56-char needle can
/// never be a substring of a ≤12-char asset code, so hash mode is provably
/// empty). Step 1 pages matching assets — `asset_code` substring or the
/// `native`/`xlm` special-case — joining the smaller `soroban_contracts` for the
/// contract StrKey, RANKED by match tier then holder count (task 0485; see the
/// statement comment). Step 2 resolves the page's issuer surrogates → G-StrKey
/// via a bloom-pruned `accounts WHERE id IN (...)` seek (NEVER a full-table
/// `accounts` join — the Code 241 trap). `route_token` is then composed in Rust.
/// The displayed code of an asset row — native's `XLM` standing in for its
/// empty stored code. See the note in the function body before changing it.
const SHOWN: &str = "lower(if(a.asset_type = 0, 'XLM', toString(a.asset_code)))";

async fn search_assets(
    client: &clickhouse::Client,
    q: &str,
    classified: &Classified,
    include: &IncludeFlags,
    per_group_limit: i32,
) -> Result<Vec<(String, SearchHit)>, clickhouse::error::Error> {
    if !include.asset || classified.hash_bytes.is_some() {
        return Ok(Vec::new());
    }

    // Step 1: page matching assets. `assets` is small (state table); `FINAL`
    // collapses re-ingested versions. `q` is bound (3×: the substring needle and
    // the two case-folded native-token comparisons).
    //
    // The `soroban_contracts` side of the join is collapsed to one row per `id`
    // (`GROUP BY id` + `any(contract_id)`) — NOT joined bare. It is a
    // ReplacingMergeTree with unmerged duplicate ids (some ×6), so a bare join
    // fans each matching asset out 2–6× into identical hits and burns the
    // `per_group_limit` budget. `contract_id` (C-StrKey) is immutable across
    // versions, so `any()` is exact.
    //
    // Grouping the WHOLE (small, ~146k-row) table beats scoping it to the page:
    // a page-scoped `IN (SELECT … FROM page)` needs the `page` CTE, and CH does
    // not materialise CTEs — it would evaluate the asset scan twice. Measured
    // (lore-0420): page-scoped CTE 1,896,766 rows / 37.8 MiB, this form
    // 1,118,154 rows / 28.5 MiB — cheaper even than the un-deduped original
    // (1,151,738 / 32.0 MiB).
    //
    // A fully-qualified `CODE:ISSUER` (task 0534) takes a different arm: the pair
    // names exactly one asset, so it is an equality lookup and needs no ranking —
    // the most precise query is also the cheapest one. The issuer StrKey resolves
    // through `accounts`, whose `ORDER BY account_id` primary key makes it a point
    // seek rather than the ~23M-row hash join that OOMs (Code 241).
    //
    // The substring arm below is where relevance lives (task 0485). Before it,
    // that arm ended in a bare `LIMIT` with NO `ORDER BY`, so it returned
    // whichever rows the scan reached first — `q=USDC` answered with ten `IUSDC`
    // rows and no USDC at all, and two identical calls could disagree.
    //
    // `SHOWN` is the code a row DISPLAYS as, and both the match and the tier
    // compare it — never the stored value. Native XLM stores an EMPTY code and
    // renders as `XLM`, so comparing what is stored returned thousands of
    // impostor codes and missed the one asset everybody meant. That is also why
    // there is no `native` arm: the alias IS the comparison.
    //
    // The same expression appears in `assets::queries` (the list) and in
    // `common::pool_asset_codes` (the legs). It was briefly a shared builder;
    // three literal copies read better than the indirection, so if you change
    // the shape here, change it there — `native_is_matched_by_type_not_by_
    // stored_code` in each module is the test that fails when you do not.
    //
    // Ranking is a tier (exact > prefix > substring anywhere) rather than a
    // scoring formula: the order follows from what matched, not from a weighting
    // we invented. Within a tier the tie-break is holder count — 441 assets carry
    // the code `USDC` and the signal separates them cleanly (691,713 holders for
    // Circle's, 3,093 for the runner-up). It lives in `balance_aggregates` (task
    // 0331) — `assets` has no holder column any more (task 0310). Joined bare:
    // the table is 1:1 on `asset_id`, so the usual `GROUP BY` collapse is pure
    // cost (measured 93 ms -> 71 ms without it), and it cannot be page-scoped
    // like the list's join because the ranking needs holders BEFORE the limit.
    //
    // The trailing PK columns make the order total: holder counts are NULL for
    // most rows, and "same query, same answer" is half of what this fixes.
    //
    // The exact arm deliberately takes none of this: a qualified pair names one
    // row, so there is nothing to rank and nothing to pay the join for.
    const ASSET_HEAD: &str = "SELECT \
            a.asset_type AS asset_type, \
            nullIf(a.asset_code, '') AS asset_code, \
            nullIf(sc.contract_id, '') AS contract_strkey, \
            a.issuer_id AS issuer_id \
         FROM assets a FINAL \
         LEFT JOIN ( \
             SELECT id, any(contract_id) AS contract_id \
             FROM soroban_contracts GROUP BY id \
         ) sc ON sc.id = a.contract_id ";
    let rows = if let Some((code, issuer)) = classified.code_issuer.as_ref() {
        let sql = format!(
            "{ASSET_HEAD} \
             WHERE lower(toString(a.asset_code)) = lower(?) \
               AND a.issuer_id IN (SELECT id FROM accounts WHERE account_id = ?) \
             LIMIT {per_group_limit}"
        );
        client
            .query(&sql)
            .bind(code)
            .bind(issuer)
            .fetch_all::<AssetPhase1Row>()
            .await?
    } else {
        let sql = format!(
            "{ASSET_HEAD} \
             LEFT JOIN balance_aggregates bagg ON bagg.asset_id = a.id \
             WHERE position({SHOWN}, lower(?)) > 0 \
             ORDER BY multiIf({SHOWN} = lower(?), 0, \
                              startsWith({SHOWN}, lower(?)), 1, \
                              2) ASC, \
                 bagg.holder_count DESC NULLS LAST, \
                 a.asset_type ASC, a.asset_code ASC, a.issuer_id ASC \
             LIMIT {per_group_limit}"
        );
        // One bind for the match, two for the tier — left to right, same needle.
        client
            .query(&sql)
            .bind(q)
            .bind(q)
            .bind(q)
            .fetch_all::<AssetPhase1Row>()
            .await?
    };
    if rows.is_empty() {
        return Ok(Vec::new());
    }

    // Step 2: resolve issuer surrogates → G-StrKey. `account_id` is immutable
    // across versions, so no FINAL; `LIMIT 1 BY id` collapses re-ingest parts.
    // `idx_acc_id` (bloom on `id`) prunes the ~23M-row table to the page keys.
    // i64 IN-list, bounded by the page, no injection surface.
    let issuer_ids: BTreeSet<i64> = rows
        .iter()
        .map(|r| r.issuer_id)
        .filter(|&i| i != 0)
        .collect();
    let issuers: HashMap<i64, String> = if issuer_ids.is_empty() {
        HashMap::new()
    } else {
        let in_list = issuer_ids
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        client
            .query(&format!(
                "SELECT id AS id, account_id AS account_id \
                 FROM accounts WHERE id IN ({in_list}) LIMIT 1 BY id"
            ))
            .fetch_all::<IssuerRow>()
            .await?
            .into_iter()
            .map(|r| (r.id, r.account_id))
            .collect()
    };

    Ok(rows
        .into_iter()
        .map(|r| {
            let issuer = (r.issuer_id != 0)
                .then(|| issuers.get(&r.issuer_id))
                .flatten()
                .map(String::as_str);
            let route_token = asset_route_token(
                r.contract_strkey.as_deref(),
                r.asset_code.as_deref(),
                issuer,
                r.asset_type,
            );
            (
                "asset".to_string(),
                SearchHit {
                    entity_type: EntityType::Asset,
                    // Display id is the asset code; native (no code) shows XLM,
                    // matching the PG `COALESCE(asset_code, 'XLM')`.
                    identifier: r.asset_code.unwrap_or_else(|| "XLM".to_string()),
                    label: asset_family_name(r.asset_type).unwrap_or_default(),
                    route_token,
                    successful: None,
                    last_activity_at: None,
                    contract_id: None,
                    token_id: None,
                },
            )
        })
        .collect())
}

// ---------------------------------------------------------------------------
// NFTs — name substring (from nft_enrichment), composite-id routing
// ---------------------------------------------------------------------------

#[derive(Debug, Row, Deserialize)]
struct NftSearchRow {
    identifier: String,
    label: String,
    contract_strkey: String,
    token_id: String,
}

/// Fires whenever the query is NOT hash-shaped. The searchable `name` lives in
/// `nft_enrichment` (NOT vestigial `nfts.name`), collapsed with
/// `argMax(_, version)`. The page is bounded by `LIMIT`, then the contract
/// surrogate is resolved → C-StrKey via a page-scoped `soroban_contracts`
/// lookup — the same structure as the live `/nfts` list. The NFT routes on the
/// `(contract_id, token_id)` composite (ADR 0030), carried on the hit's
/// `contract_id` / `token_id` fields; `route_token` stays `None`.
async fn search_nfts(
    client: &clickhouse::Client,
    q: &str,
    classified: &Classified,
    include: &IncludeFlags,
    per_group_limit: i32,
) -> Result<Vec<(String, SearchHit)>, clickhouse::error::Error> {
    if !include.nft || classified.hash_bytes.is_some() {
        return Ok(Vec::new());
    }

    let sql = format!(
        "WITH \
         enr AS ( \
             SELECT contract_id, token_id, \
                    argMax(name, version)            AS name, \
                    argMax(collection_name, version) AS collection_name \
             FROM nft_enrichment GROUP BY contract_id, token_id \
         ), \
         page AS ( \
             SELECT n.contract_id     AS contract_surrogate, \
                    n.token_id        AS token_id, \
                    e.name            AS e_name, \
                    e.collection_name AS e_collection_name \
             FROM nfts n FINAL \
             LEFT JOIN enr e ON e.contract_id = n.contract_id AND e.token_id = n.token_id \
             WHERE positionCaseInsensitive(ifNull(e.name, ''), ?) > 0 \
             LIMIT {per_group_limit} \
         ), \
         sc AS ( \
             SELECT id, any(contract_id) AS contract_id \
             FROM soroban_contracts \
             WHERE id IN (SELECT contract_surrogate FROM page) GROUP BY id \
         ), \
         scm AS ( \
             SELECT contract_id, argMax(name, version) AS name \
             FROM soroban_contract_metadata \
             WHERE contract_id IN (SELECT contract_id FROM sc) \
             GROUP BY contract_id \
         ) \
         SELECT \
             ifNull(p.e_name, '')            AS identifier, \
             ifNull(coalesce(nullIf(scm.name, ''), nullIf(p.e_collection_name, '')), '') AS label, \
             sc.contract_id                  AS contract_strkey, \
             p.token_id                      AS token_id \
         FROM page p INNER JOIN sc ON sc.id = p.contract_surrogate LEFT JOIN scm ON scm.contract_id = sc.contract_id"
    );
    let rows = client
        .query(&sql)
        .bind(q)
        .fetch_all::<NftSearchRow>()
        .await?;

    Ok(rows
        .into_iter()
        .map(|r| {
            (
                "nft".to_string(),
                SearchHit {
                    entity_type: EntityType::Nft,
                    identifier: r.identifier,
                    label: r.label,
                    route_token: None,
                    successful: None,
                    last_activity_at: None,
                    contract_id: Some(r.contract_strkey),
                    token_id: Some(r.token_id),
                },
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_family_name_matches_pg_function() {
        assert_eq!(asset_family_name(0).as_deref(), Some("native"));
        assert_eq!(asset_family_name(1).as_deref(), Some("classic_credit"));
        // 2 (`sac`) retired — ADR 0051.
        assert_eq!(asset_family_name(2), None);
        assert_eq!(asset_family_name(3).as_deref(), Some("soroban"));
        assert_eq!(asset_family_name(99), None);
    }

    #[test]
    fn route_token_prefers_contract_strkey() {
        assert_eq!(
            asset_route_token(Some("CABC"), Some("USDC"), Some("GISS"), 1).as_deref(),
            Some("CABC"),
        );
    }

    #[test]
    fn route_token_falls_back_to_code_issuer() {
        assert_eq!(
            asset_route_token(None, Some("USDC"), Some("GISS"), 1).as_deref(),
            Some("USDC-GISS"),
        );
        // Empty contract strkey is treated as absent (nullIf sentinel parity).
        assert_eq!(
            asset_route_token(Some(""), Some("USDC"), Some("GISS"), 1).as_deref(),
            Some("USDC-GISS"),
        );
    }

    #[test]
    fn route_token_native_only_when_type_zero() {
        assert_eq!(
            asset_route_token(None, None, None, 0).as_deref(),
            Some("native"),
        );
        // Code present but issuer unresolved on a non-native row → honest None
        // (never a mis-route to the native page).
        assert_eq!(asset_route_token(None, Some("USDC"), None, 1), None);
    }
}

/// Live-CH decode smoke for the search read path. The curl `FORMAT` box smokes
/// do NOT exercise the clickhouse-rs RowBinary decoder, so a wire-type↔struct
/// mismatch (e.g. a Nullable column decoded into a non-Option field, or a
/// positional reorder) passes a curl check yet 500s the live endpoint. This
/// decodes rows a real CH produced for every bucket's Row struct.
///
/// **Skips cleanly when `CH_URL` is unset**, so CI (no CH access) is green.
/// Run against a reachable CH (local replica or SSH tunnel):
///
/// ```text
/// CH_URL=http://127.0.0.1:8123 CH_DATABASE=default \
///   cargo test -p api --lib search::queries::decode_smoke -- --nocapture
/// ```
#[cfg(test)]
mod decode_smoke {
    use super::super::classifier;
    use super::*;

    #[derive(Debug, Row, Deserialize)]
    struct HashHexRow {
        hash_hex: String,
    }

    fn client() -> Option<clickhouse::Client> {
        let url = std::env::var("CH_URL").ok()?;
        let mut c = clickhouse::Client::default().with_url(url);
        if let Ok(u) = std::env::var("CH_USER") {
            c = c.with_user(u);
        }
        if let Ok(p) = std::env::var("CH_PASSWORD") {
            c = c.with_password(p);
        }
        if let Ok(d) = std::env::var("CH_DATABASE") {
            c = c.with_database(d);
        }
        Some(c)
    }

    /// Every search bucket's Row struct must decode the rows a real CH emits.
    /// Text / prefix modes exercise account / contract / asset / nft on any
    /// populated CH; a bootstrapped real hash exercises transaction + pool.
    #[tokio::test]
    async fn search_ch_rows_decode() {
        let Some(ch) = client() else {
            eprintln!("CH_URL unset — skipping search CH decode smoke");
            return;
        };
        let all = IncludeFlags::all();

        // Text mode → contract(name) + asset + nft. Prefix mode → account +
        // contract(prefix) + asset + nft. Both decode-exercise their structs
        // regardless of whether the corpus yields rows.
        for q in ["a", "GA", "CA"] {
            fetch_search(&ch, q, &classifier::classify(q), &all, 5)
                .await
                .unwrap_or_else(|e| panic!("search decode failed for q={q:?}: {e}"));
        }

        // Bootstrap a real tx hash so the transaction + pool buckets decode.
        let boot = ch
            .query("SELECT lower(hex(hash)) AS hash_hex FROM transaction_hash_index LIMIT 1")
            .fetch_optional::<HashHexRow>()
            .await
            .expect("bootstrap hash query must run");
        let Some(boot) = boot else {
            eprintln!("transaction_hash_index empty — text-mode decode ok, skipping hash mode");
            return;
        };
        fetch_search(
            &ch,
            &boot.hash_hex,
            &classifier::classify(&boot.hash_hex),
            &all,
            5,
        )
        .await
        .expect("transaction/pool bucket rows must decode");
    }

    /// Task 0485. The tier ranking is only visible in the ORDER of the rows,
    /// so the SQL-shape tests cannot see it — this runs the real read and
    /// looks at the first asset hit. It also exercises the statement's 7
    /// placeholders against the 7 `.bind(q)` calls; a mismatch is a runtime
    /// failure no offline test reaches.
    #[tokio::test]
    async fn native_xlm_is_the_first_asset_hit_for_xlm() {
        let Some(ch) = client() else {
            eprintln!("CH_URL unset — skipping asset ranking smoke");
            return;
        };
        let has_native: u64 = ch
            .query("SELECT count() FROM assets WHERE asset_type = 0")
            .fetch_one()
            .await
            .expect("native probe must run");
        if has_native == 0 {
            eprintln!("no native row in this CH — ranking smoke not exercised");
            return;
        }

        let all = IncludeFlags::all();
        let hits = fetch_search(&ch, "xlm", &classifier::classify("xlm"), &all, 20)
            .await
            .expect("ranked asset search decodes");

        let first = hits
            .iter()
            .find(|(bucket, _)| bucket == "asset")
            .map(|(_, hit)| hit)
            .expect("a corpus with native XLM must yield an asset hit for `xlm`");
        assert_eq!(
            first.label, "native",
            "`xlm` answered with {:?} first — before the ranking this bucket \
             returned whichever look-alike codes the scan reached first and \
             native XLM never made the page",
            first.identifier
        );
    }

    /// A needle longer than a Stellar asset code cannot match a pool, so the
    /// scan must not run at all. Guards the gate that keeps every account- and
    /// contract-shaped search off the pools table (task 0470 review).
    #[tokio::test]
    async fn a_strkey_shaped_query_never_scans_the_pools_table() {
        let Some(ch) = client() else {
            eprintln!("CH_URL unset — skipping pools shape-gate smoke");
            return;
        };
        let all = IncludeFlags::all();

        // 56 characters: an account StrKey. Classified as a prefix, not a
        // hash, so before the gate this fell through to the code arm.
        let q = "GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN";
        let hits = fetch_search(&ch, q, &classifier::classify(q), &all, 20)
            .await
            .expect("search decodes");
        assert!(
            !hits.iter().any(|(bucket, _)| bucket == "pool"),
            "a StrKey-shaped query must not reach the pools bucket"
        );
    }

    /// Task 0470: a non-hash query used to match no pool at all, so an asset
    /// code returned zero here while `/v1/liquidity-pools` returned dozens —
    /// which read as the 0440 fix not working.
    ///
    /// This does NOT re-test the matching rule: both surfaces call
    /// `common::pool_asset_codes::asset_codes_predicate`, so they cannot
    /// disagree, and that module's unit tests pin the pair semantics and the
    /// native arm. What is only testable against a real ClickHouse is the rest
    /// of this arm — that the grouped subquery parses, that `PoolRow` decodes
    /// it, and that a plain asset code reaches pools at all.
    #[tokio::test]
    async fn search_matches_pools_by_asset_code() {
        let Some(ch) = client() else {
            eprintln!("CH_URL unset — skipping search pool asset-code smoke");
            return;
        };
        let all = IncludeFlags::all();

        // `XLM` is the needle that separates a correct predicate from a
        // plausible one: native legs carry an empty stored code, so a column
        // match without the `if(type = 0, …)` arm returns credit assets minted
        // under the code `XLM` and misses every real native pool.
        for needle in ["XLM", "USD"] {
            let hits = fetch_search(&ch, needle, &classifier::classify(needle), &all, 20)
                .await
                .expect("search decodes");
            let pools: Vec<&SearchHit> = hits
                .iter()
                .filter(|(bucket, _)| bucket == "pool")
                .map(|(_, hit)| hit)
                .collect();

            assert!(
                !pools.is_empty(),
                "search returned no pool for {needle:?} — the asset-code arm regressed to id-only"
            );
            for hit in pools {
                assert!(
                    hit.label.to_uppercase().contains(needle),
                    "pool {} labelled {:?} matches neither leg of {needle:?} — filter not applied",
                    hit.identifier,
                    hit.label,
                );
                assert!(
                    hit.identifier.starts_with('L'),
                    "pool identifier {:?} is not an L-strkey",
                    hit.identifier,
                );
            }
        }
    }

    /// Task 0485: the canonical `CODE:ISSUER` (and our own `CODE-ISSUER` route
    /// token) used to classify as nothing, so the asset arm hunted a 60+
    /// character needle through <=12 character codes — provably empty — and the
    /// most precise query a user can type answered with a blank page.
    ///
    /// Only testable against a real corpus: what is asserted is which row
    /// survives, which no fixture-free unit test can observe. The target is a
    /// code carried by SEVERAL assets, addressed by the one the scan reaches
    /// LAST, so a hit cannot come from the substring arm returning the first row
    /// it happened to touch.
    #[tokio::test]
    async fn code_issuer_resolves_to_exactly_that_asset() {
        let Some(ch) = client() else {
            eprintln!("CH_URL unset — skipping CODE:ISSUER smoke");
            return;
        };

        #[derive(Debug, Row, Deserialize)]
        struct CodeRow {
            asset_code: String,
        }

        let probe = ch
            .query(
                "SELECT toString(a.asset_code) AS asset_code \
                 FROM assets a FINAL \
                 WHERE length(a.asset_code) > 0 AND a.issuer_id != 0 \
                 GROUP BY a.asset_code \
                 HAVING count() > 1 \
                 LIMIT 1",
            )
            .fetch_optional::<CodeRow>()
            .await
            .expect("corpus probe must run");
        let Some(CodeRow { asset_code: code }) = probe else {
            eprintln!("no asset code shared by two issuers — skipping");
            return;
        };

        let same_code = ch
            .query(
                "SELECT a.asset_type AS asset_type, \
                        nullIf(a.asset_code, '') AS asset_code, \
                        nullIf(sc.contract_id, '') AS contract_strkey, \
                        a.issuer_id AS issuer_id \
                 FROM assets a FINAL \
                 LEFT JOIN ( \
                     SELECT id, any(contract_id) AS contract_id \
                     FROM soroban_contracts GROUP BY id \
                 ) sc ON sc.id = a.contract_id \
                 WHERE lower(toString(a.asset_code)) = lower(?) \
                 LIMIT 16",
            )
            .bind(&code)
            .fetch_all::<AssetPhase1Row>()
            .await
            .expect("same-code query must run");
        let target = same_code
            .last()
            .unwrap_or_else(|| panic!("no asset row for probed code {code:?}"));

        let Some(issuer) = ch
            .query(
                "SELECT id AS id, account_id AS account_id \
                 FROM accounts WHERE id = ? LIMIT 1 BY id",
            )
            .bind(target.issuer_id)
            .fetch_optional::<IssuerRow>()
            .await
            .expect("issuer resolve must run")
            .map(|r| r.account_id)
        else {
            eprintln!("probed asset has no resolvable issuer — skipping");
            return;
        };

        let want = asset_route_token(
            target.contract_strkey.as_deref(),
            target.asset_code.as_deref(),
            Some(issuer.as_str()),
            target.asset_type,
        );

        // Both separators: `:` is the canonical SEP / SDK form, `-` is what our
        // own `/assets/:id` routes emit and users paste back.
        for q in [format!("{code}:{issuer}"), format!("{code}-{issuer}")] {
            let hits = fetch_search(&ch, &q, &classifier::classify(&q), &IncludeFlags::all(), 10)
                .await
                .unwrap_or_else(|e| panic!("search failed for {q:?}: {e}"));
            let assets: Vec<&SearchHit> = hits
                .iter()
                .filter(|(bucket, _)| bucket == "asset")
                .map(|(_, hit)| hit)
                .collect();

            assert_eq!(
                assets.len(),
                1,
                "{q:?} must resolve to exactly one asset, got {assets:?}"
            );
            assert_eq!(
                assets[0].route_token, want,
                "{q:?} resolved to the wrong asset",
            );
        }
    }
}
