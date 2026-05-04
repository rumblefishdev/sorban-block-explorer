//! Database queries backing `GET /v1/search`.
//!
//! Implementation of the canonical SQL in
//! `docs/architecture/database-schema/endpoint-queries/22_get_search.sql`.
//!
//! Two queries:
//!
//! * [`fetch_redirect`] — exact-match short-circuit when `q` is a
//!   fully-typed entity id (64-hex hash, full G-StrKey, full C-StrKey).
//!   Returns `Some(EntityType, identifier)` for the first table that
//!   matches, in priority order, or `None` for fall-through to broad
//!   search.
//! * [`fetch_search`] — runs the union-of-CTEs broad-search statement
//!   with the per-bucket `:include_*` flags resolved from the optional
//!   `?type=` filter.

use sqlx::{PgPool, Row};

use super::classifier::Classified;
use super::dto::{EntityType, SearchHit};

// ---------------------------------------------------------------------------
// Type-filter map
// ---------------------------------------------------------------------------

/// Per-entity inclusion flags. Defaults to "include everything"; a
/// caller-supplied `?type=foo,bar` filter narrows it before the query
/// runs.
#[derive(Debug, Clone, Copy)]
pub struct IncludeFlags {
    pub tx: bool,
    pub contract: bool,
    pub asset: bool,
    pub account: bool,
    pub nft: bool,
    pub pool: bool,
}

impl IncludeFlags {
    pub fn all() -> Self {
        Self {
            tx: true,
            contract: true,
            asset: true,
            account: true,
            nft: true,
            pool: true,
        }
    }

    pub fn none() -> Self {
        Self {
            tx: false,
            contract: false,
            asset: false,
            account: false,
            nft: false,
            pool: false,
        }
    }

    pub fn enable(&mut self, t: EntityType) {
        match t {
            EntityType::Transaction => self.tx = true,
            EntityType::Contract => self.contract = true,
            EntityType::Asset => self.asset = true,
            EntityType::Account => self.account = true,
            EntityType::Nft => self.nft = true,
            EntityType::Pool => self.pool = true,
        }
    }
}

// ---------------------------------------------------------------------------
// Redirect short-circuit
// ---------------------------------------------------------------------------

/// Look up the first entity that matches `q` exactly. Used when the
/// classifier flags `q` as fully-typed — frontend wants to skip the
/// dropdown and navigate directly.
///
/// Priority (matches the human "what was the user looking for?"
/// expectation when an id-shape value is ambiguous):
///   1. transaction hash (32-byte BYTEA)
///   2. liquidity pool id (also 32-byte BYTEA — same `hash_bytes`)
///   3. account StrKey (full 56 chars, `G…`)
///   4. soroban contract StrKey (full 56 chars, `C…`)
///
/// Returns `None` if no exact row matches; the caller falls through to
/// broad search so the user still sees suggestions instead of a 404.
pub async fn fetch_redirect(
    pool: &PgPool,
    classified: &Classified,
) -> Result<Option<(EntityType, String)>, sqlx::Error> {
    if !classified.is_fully_typed {
        return Ok(None);
    }

    if let Some(bytes) = &classified.hash_bytes {
        let tx: Option<(String,)> = sqlx::query_as(
            "SELECT encode(hash, 'hex') FROM transaction_hash_index WHERE hash = $1",
        )
        .bind(bytes.as_slice())
        .fetch_optional(pool)
        .await?;
        if let Some((id,)) = tx {
            return Ok(Some((EntityType::Transaction, id)));
        }

        let pool_row: Option<(String,)> =
            sqlx::query_as("SELECT encode(pool_id, 'hex') FROM liquidity_pools WHERE pool_id = $1")
                .bind(bytes.as_slice())
                .fetch_optional(pool)
                .await?;
        if let Some((id,)) = pool_row {
            return Ok(Some((EntityType::Pool, id)));
        }
        return Ok(None);
    }

    if let Some(strkey) = &classified.strkey_prefix
        && strkey.len() == 56
    {
        match strkey.as_bytes()[0] {
            b'G' => {
                let row: Option<(String,)> =
                    sqlx::query_as("SELECT account_id FROM accounts WHERE account_id = $1")
                        .bind(strkey)
                        .fetch_optional(pool)
                        .await?;
                if let Some((id,)) = row {
                    return Ok(Some((EntityType::Account, id)));
                }
            }
            b'C' => {
                let row: Option<(String,)> = sqlx::query_as(
                    "SELECT contract_id FROM soroban_contracts WHERE contract_id = $1",
                )
                .bind(strkey)
                .fetch_optional(pool)
                .await?;
                if let Some((id,)) = row {
                    return Ok(Some((EntityType::Contract, id)));
                }
            }
            _ => {}
        }
    }

    Ok(None)
}

// ---------------------------------------------------------------------------
// Broad search
// ---------------------------------------------------------------------------

/// Run the canonical `22_get_search.sql` UNION of six narrow CTEs and
/// return the rows partitioned by `entity_type`. The caller groups
/// these into [`SearchGroups`](super::dto::SearchGroups) for the JSON
/// response.
pub async fn fetch_search(
    pool: &PgPool,
    q: &str,
    classified: &Classified,
    include: &IncludeFlags,
    per_group_limit: i32,
) -> Result<Vec<(String, SearchHit)>, sqlx::Error> {
    // Verbatim port of `docs/architecture/database-schema/endpoint-queries/22_get_search.sql`.
    let sql = r#"
        WITH
        tx_hits AS (
            SELECT
                'transaction'::text       AS entity_type,
                encode(thi.hash, 'hex')   AS identifier,
                'ledger ' || thi.ledger_sequence::text AS label,
                NULL::bigint              AS surrogate_id
            FROM transaction_hash_index thi
            WHERE $5  = TRUE
              AND $2 IS NOT NULL
              AND thi.hash = $2
            LIMIT $4
        ),
        contract_hits AS (
            SELECT
                'contract'::text          AS entity_type,
                sc.contract_id            AS identifier,
                COALESCE(sc.metadata->>'name', '')  AS label,
                sc.id                     AS surrogate_id
            FROM soroban_contracts sc
            WHERE $6 = TRUE
              AND (
                      ( $3 IS NOT NULL AND sc.contract_id LIKE $3 || '%' )
                   OR ( $3 IS NULL     AND sc.search_vector @@ plainto_tsquery('simple', $1) )
                  )
            LIMIT $4
        ),
        asset_hits AS (
            SELECT
                'asset'::text                       AS entity_type,
                COALESCE(a.asset_code, 'XLM')       AS identifier,
                token_asset_type_name(a.asset_type) AS label,
                a.id::bigint                        AS surrogate_id
            FROM assets a
            WHERE $7 = TRUE
              AND (
                      (a.asset_code IS NOT NULL AND a.asset_code ILIKE '%' || $1 || '%')
                   OR (a.asset_type = 0 AND ($1 ILIKE 'xlm' OR $1 ILIKE 'native'))
                  )
            LIMIT $4
        ),
        account_hits AS (
            SELECT
                'account'::text         AS entity_type,
                a.account_id            AS identifier,
                COALESCE(a.home_domain, '') AS label,
                a.id                    AS surrogate_id
            FROM accounts a
            WHERE $8 = TRUE
              AND $3 IS NOT NULL
              AND a.account_id LIKE $3 || '%'
            LIMIT $4
        ),
        nft_hits AS (
            SELECT
                'nft'::text                          AS entity_type,
                n.name                               AS identifier,
                COALESCE(n.collection_name, '')      AS label,
                n.id::bigint                         AS surrogate_id
            FROM nfts n
            WHERE $9 = TRUE
              AND n.name IS NOT NULL
              AND n.name ILIKE '%' || $1 || '%'
            LIMIT $4
        ),
        pool_hits AS (
            SELECT
                'pool'::text                AS entity_type,
                encode(lp.pool_id, 'hex')   AS identifier,
                (
                    COALESCE(lp.asset_a_code, 'XLM')
                    || ' / '
                    || COALESCE(lp.asset_b_code, 'XLM')
                )::text                     AS label,
                NULL::bigint                AS surrogate_id
            FROM liquidity_pools lp
            WHERE $10 = TRUE
              AND $2 IS NOT NULL
              AND lp.pool_id = $2
            LIMIT $4
        )
        SELECT entity_type, identifier, label, surrogate_id FROM tx_hits
        UNION ALL
        SELECT entity_type, identifier, label, surrogate_id FROM contract_hits
        UNION ALL
        SELECT entity_type, identifier, label, surrogate_id FROM asset_hits
        UNION ALL
        SELECT entity_type, identifier, label, surrogate_id FROM account_hits
        UNION ALL
        SELECT entity_type, identifier, label, surrogate_id FROM nft_hits
        UNION ALL
        SELECT entity_type, identifier, label, surrogate_id FROM pool_hits
    "#;

    let rows = sqlx::query(sql)
        .bind(q)
        .bind(classified.hash_bytes.as_deref())
        .bind(classified.strkey_prefix.as_deref())
        .bind(per_group_limit)
        .bind(include.tx)
        .bind(include.contract)
        .bind(include.asset)
        .bind(include.account)
        .bind(include.nft)
        .bind(include.pool)
        .fetch_all(pool)
        .await?;

    let hits = rows
        .into_iter()
        .filter_map(|row| {
            let entity_type: String = row.get("entity_type");
            let parsed = match EntityType::parse(&entity_type) {
                Some(p) => p,
                None => {
                    tracing::error!(
                        entity_type = entity_type.as_str(),
                        "search SQL emitted unknown entity_type literal — \
                         22_get_search.sql out of sync with EntityType enum",
                    );
                    return None;
                }
            };
            let identifier: String = row.get("identifier");
            let label: String = row.get("label");
            let surrogate_id: Option<i64> = row.get("surrogate_id");
            Some((
                entity_type,
                SearchHit {
                    entity_type: parsed,
                    identifier,
                    label,
                    surrogate_id,
                },
            ))
        })
        .collect();

    Ok(hits)
}
