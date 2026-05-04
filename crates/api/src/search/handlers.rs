//! Axum handler for `GET /v1/search`.

#![allow(clippy::result_large_err)]

use axum::Json;
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::common::errors;
use crate::openapi::schemas::ErrorEnvelope;
use crate::state::AppState;

use super::classifier;
use super::dto::{
    EntityType, SearchGroups, SearchHit, SearchRedirect, SearchResponse, SearchResults,
};
use super::queries::{self, IncludeFlags};

/// Default per-group cap when caller omits `?limit=` (matches
/// `22_get_search.sql` recommendation).
const DEFAULT_LIMIT: u32 = 10;

/// Hard ceiling on per-group cap. Kept low — broad search runs six
/// CTEs in a single statement, so a high `limit` multiplies index
/// reads. 50 is enough for one dropdown page on every entity bucket.
const MAX_LIMIT: u32 = 50;

/// Hard ceiling on `?q=` length. Defence in depth — Lambda payload
/// limits will eventually clip absurd inputs, but a 256-byte cap keeps
/// the trigram / FTS scan bounded and rejects garbage early at the
/// edge of the request lifecycle. The longest legitimate input is a
/// full StrKey (56 chars) or a 64-char hex hash; 256 leaves headroom
/// for future identifier shapes without inviting `q=<10MB blob>`.
const MAX_Q_LEN: usize = 256;

/// Raw query-string shape. Accept all values as `String` so we can
/// emit precise canonical error codes instead of serde's generic 422.
#[derive(Debug, Deserialize)]
pub struct SearchParams {
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default, rename = "type")]
    pub r#type: Option<String>,
    #[serde(default)]
    pub limit: Option<String>,
}

/// Unified search across all entity types.
///
/// `?q=` is required. `?type=` (CSV) restricts the result to specific
/// entity types — values must be in the closed allowlist
/// (`transaction`, `contract`, `asset`, `account`, `nft`, `pool`).
/// `?limit=` caps each entity bucket independently (default 10,
/// ceiling 50).
///
/// Behaviour:
/// * If `q` is a fully-typed entity id (64-hex hash, full G-StrKey,
///   full C-StrKey) and the corresponding entity exists, the response
///   is `{ "type": "redirect", "entity_type", "entity_id" }` — frontend
///   navigates directly.
/// * Otherwise the response is `{ "type": "results", "groups": {...} }`
///   with up to `limit` rows per entity bucket. Rows carry the same
///   four columns regardless of bucket: `entity_type`, `identifier`,
///   `label`, `surrogate_id` (BIGINT FK or `null`).
///
/// Authoritative SQL:
/// `docs/architecture/database-schema/endpoint-queries/22_get_search.sql`.
#[utoipa::path(
    get,
    path = "/search",
    tag = "search",
    params(
        ("q" = String, Query, description = "Search query string. Required, non-empty after trim."),
        ("type" = Option<String>, Query,
            description = "CSV of entity types to include. Allowed: transaction, contract, asset, account, nft, pool."),
        ("limit" = Option<u32>, Query,
            description = "Per-group result cap. Default 10, max 50.",
            minimum = 1, maximum = 50),
    ),
    responses(
        (status = 200, description = "Search results", body = SearchResponse),
        (status = 400, description = "Validation error", body = ErrorEnvelope),
        (status = 500, description = "Database error", body = ErrorEnvelope),
    ),
)]
pub async fn get_search(
    State(state): State<AppState>,
    Query(params): Query<SearchParams>,
) -> Response {
    // 1. Validate `q` — required, non-empty after trim.
    let q_raw = match params.q.as_deref() {
        Some(s) => s.trim(),
        None => {
            return errors::bad_request(
                errors::INVALID_SEARCH_QUERY,
                "Search query 'q' parameter is required.",
            );
        }
    };
    if q_raw.is_empty() {
        return errors::bad_request(
            errors::INVALID_SEARCH_QUERY,
            "Search query 'q' parameter is required.",
        );
    }
    if q_raw.len() > MAX_Q_LEN {
        return errors::bad_request_with_details(
            errors::INVALID_SEARCH_QUERY,
            format!("Search query 'q' must be at most {MAX_Q_LEN} bytes."),
            serde_json::json!({ "max": MAX_Q_LEN, "received_len": q_raw.len() }),
        );
    }

    // 2. Validate `?type=` filter and build IncludeFlags.
    let include = match parse_type_filter(params.r#type.as_deref()) {
        Ok(f) => f,
        Err(resp) => return resp,
    };

    // 3. Validate `?limit=`.
    let limit = match parse_limit(params.limit.as_deref()) {
        Ok(l) => l,
        Err(resp) => return resp,
    };

    // 4. Classify query.
    let classified = classifier::classify(q_raw);

    // 5. Redirect short-circuit when `q` is a fully-typed entity id
    //    that hits an existing row.
    match queries::fetch_redirect(&state.db, &classified).await {
        Ok(Some((entity_type, entity_id))) => {
            return Json(SearchResponse::Redirect(SearchRedirect {
                entity_type,
                entity_id,
            }))
            .into_response();
        }
        Ok(None) => {}
        Err(e) => {
            tracing::error!("DB error in get_search redirect: {e}");
            return errors::internal_error(errors::DB_ERROR, "Unable to perform search.");
        }
    }

    // 6. Broad search.
    let rows =
        match queries::fetch_search(&state.db, q_raw, &classified, &include, limit as i32).await {
            Ok(rows) => rows,
            Err(e) => {
                tracing::error!("DB error in get_search broad: {e}");
                return errors::internal_error(errors::DB_ERROR, "Unable to perform search.");
            }
        };

    let groups = group_hits(rows);
    Json(SearchResponse::Results(SearchResults { groups })).into_response()
}

/// Parse the optional CSV `?type=` filter.
///
/// Empty string and missing filter both mean "include everything".
/// Whitespace around values is trimmed; empty entries (`,,`) are
/// ignored. Any unknown value rejects the request with
/// `invalid_search_type` and the offending token in `details`.
fn parse_type_filter(raw: Option<&str>) -> Result<IncludeFlags, Response> {
    let Some(s) = raw else {
        return Ok(IncludeFlags::all());
    };
    let s = s.trim();
    if s.is_empty() {
        return Ok(IncludeFlags::all());
    }

    let mut flags = IncludeFlags::none();
    let mut any = false;
    for token in s.split(',') {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        let Some(t) = EntityType::parse(token) else {
            return Err(errors::bad_request_with_details(
                errors::INVALID_SEARCH_TYPE,
                "Invalid type filter. Allowed values: transaction, contract, asset, account, nft, pool",
                serde_json::json!({ "received": token }),
            ));
        };
        flags.enable(t);
        any = true;
    }
    // Caller passed only commas / whitespace — treat as no filter.
    if !any {
        return Ok(IncludeFlags::all());
    }
    Ok(flags)
}

fn parse_limit(raw: Option<&str>) -> Result<u32, Response> {
    let Some(s) = raw else {
        return Ok(DEFAULT_LIMIT);
    };
    let parsed: u32 = s.parse().map_err(|_| {
        errors::bad_request_with_details(
            errors::INVALID_LIMIT,
            format!("limit must be an integer between 1 and {MAX_LIMIT}"),
            serde_json::json!({ "min": 1, "max": MAX_LIMIT, "received": s }),
        )
    })?;
    if parsed == 0 || parsed > MAX_LIMIT {
        return Err(errors::bad_request_with_details(
            errors::INVALID_LIMIT,
            format!("limit must be between 1 and {MAX_LIMIT}"),
            serde_json::json!({ "min": 1, "max": MAX_LIMIT, "received": parsed }),
        ));
    }
    Ok(parsed)
}

/// Partition rows from the union query into per-entity buckets.
fn group_hits(rows: Vec<(String, SearchHit)>) -> SearchGroups {
    let mut g = SearchGroups::default();
    for (entity_type, hit) in rows {
        match entity_type.as_str() {
            "transaction" => g.transactions.push(hit),
            "contract" => g.contracts.push(hit),
            "asset" => g.assets.push(hit),
            "account" => g.accounts.push(hit),
            "nft" => g.nfts.push(hit),
            "pool" => g.pools.push(hit),
            // 22_get_search.sql emits only the closed set above; an
            // unexpected literal is a query-shape regression, not a
            // runtime branch we need to handle.
            other => tracing::error!(entity_type = other, "unknown entity_type in search row"),
        }
    }
    g
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_type_filter_accepts_csv() {
        let f = parse_type_filter(Some("transaction,contract,asset")).unwrap();
        assert!(f.tx);
        assert!(f.contract);
        assert!(f.asset);
        assert!(!f.account);
        assert!(!f.nft);
        assert!(!f.pool);
    }

    #[test]
    fn parse_type_filter_missing_includes_all() {
        let f = parse_type_filter(None).unwrap();
        assert!(f.tx && f.contract && f.asset && f.account && f.nft && f.pool);
    }

    #[test]
    fn parse_type_filter_empty_string_includes_all() {
        let f = parse_type_filter(Some("")).unwrap();
        assert!(f.tx && f.contract && f.asset && f.account && f.nft && f.pool);
    }

    #[test]
    fn parse_type_filter_rejects_unknown_value() {
        let err = parse_type_filter(Some("transaction,foobar")).unwrap_err();
        assert_eq!(err.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[test]
    fn parse_type_filter_tolerates_whitespace_and_empty_tokens() {
        let f = parse_type_filter(Some("  transaction , , account ")).unwrap();
        assert!(f.tx);
        assert!(f.account);
        assert!(!f.contract);
    }

    #[test]
    fn parse_limit_default_when_missing() {
        assert_eq!(parse_limit(None).unwrap(), DEFAULT_LIMIT);
    }

    #[test]
    fn parse_limit_accepts_in_range() {
        assert_eq!(parse_limit(Some("25")).unwrap(), 25);
        assert_eq!(parse_limit(Some("50")).unwrap(), 50);
        assert_eq!(parse_limit(Some("1")).unwrap(), 1);
    }

    #[test]
    fn parse_limit_rejects_zero_or_above_ceiling() {
        assert!(parse_limit(Some("0")).is_err());
        assert!(parse_limit(Some("51")).is_err());
        assert!(parse_limit(Some("100")).is_err());
    }

    #[test]
    fn parse_limit_rejects_non_numeric() {
        assert!(parse_limit(Some("ten")).is_err());
    }

    /// Length-cap guard for `q` is enforced inline in `get_search`, so
    /// we lock the constant at compile time rather than test the handler
    /// directly. This catches a careless drop of `MAX_Q_LEN` to an
    /// unsafe value before the test suite even runs.
    const _: () = {
        assert!(MAX_Q_LEN <= 1024, "q length cap must stay tight");
        assert!(
            MAX_Q_LEN >= 64,
            "q length cap must accommodate full 64-hex hashes"
        );
    };
}
