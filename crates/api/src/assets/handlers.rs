//! Axum handlers for the assets endpoints. Pure DB — no read-time XDR.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use domain::AssetFamily;

use crate::common::cache_control;
use crate::common::cursor;
use crate::common::cursor::Direction;
use crate::common::errors;
use crate::common::extractors::Pagination;
use crate::common::filters;
use crate::common::pagination::{finalize_page, into_envelope};
use crate::common::strkey::is_strkey_shape;
use crate::openapi::schemas::{ErrorEnvelope, PageInfo, Paginated};
use crate::runtime_enrichment::sep1::{Sep1Currency, Sep1TomlParsed};
use crate::state::AppState;
use crate::transactions::dto::TxListCursor;

use super::dto::{
    AssetDetailResponse, AssetItem, AssetKeyCursor, AssetTransactionItem, ListParams,
};
use super::queries::{self, AssetRow, AssetTxRow, ResolvedListParams};

async fn fetch_list_for_source(
    state: &AppState,
    params: &ResolvedListParams,
    direction: Direction,
) -> Result<Vec<AssetRow>, clickhouse::error::Error> {
    queries::fetch_list(&state.ch(), params, direction).await
}

/// `:id` detail-row dispatch — resolves the `AssetRow` from CH.
async fn fetch_asset_row_for_source(
    state: &AppState,
    parsed: AssetIdRef<'_>,
) -> Result<Option<AssetRow>, clickhouse::error::Error> {
    match parsed {
        AssetIdRef::Native => queries::fetch_native(&state.ch()).await,
        AssetIdRef::Contract(c) => queries::fetch_by_contract_id(&state.ch(), c).await,
        AssetIdRef::CodeIssuer(code, issuer) => {
            queries::fetch_by_code_issuer(&state.ch(), code, issuer).await
        }
    }
}

/// Canonical wire id — the single token usable as `/assets/{id}`: the contract
/// StrKey for a `soroban` asset (the contract IS the asset), the reserved
/// `native` token for XLM, else the `CODE-ISSUER` composite (classic credit).
/// A SAC-wrapped classic / native asset keys off CODE-ISSUER / `native` (ADR
/// 0051 — the SAC handle is a facet, not the identity), so the empty fallback
/// is unreachable in practice.
fn canonical_id(row: &AssetRow) -> String {
    if let Some(contract_id) = &row.contract_id {
        return contract_id.clone();
    }
    if let (Some(code), Some(issuer)) = (&row.asset_code, &row.issuer) {
        return format!("{code}-{issuer}");
    }
    // Native XLM singleton (asset_type 0): no composite identity → reserved token.
    if row.asset_type == 0 {
        return "native".to_string();
    }
    String::new()
}

/// Map a fetched row to the wire item, re-deriving the SAC `C…` StrKey on read
/// (ADR 0051 — never stored) from `code:issuer` when the asset carries an
/// observed SAC facet (`sac_contract_surrogate != 0`).
fn map_item(row: AssetRow, network_id: &[u8; 32]) -> AssetItem {
    let (sac_contract_id, sac_deployed) = if row.sac_contract_surrogate != 0 {
        let code = row.asset_code.as_deref().unwrap_or("");
        let issuer = row.issuer.as_deref().unwrap_or("");
        (
            xdr_parser::derive_sac_strkey(code, issuer, network_id),
            Some(row.sac_deployed),
        )
    } else {
        (None, None)
    };
    AssetItem {
        id: canonical_id(&row),
        asset_type_name: row.asset_type_name,
        asset_type: row.asset_type,
        asset_code: row.asset_code,
        issuer: row.issuer,
        issuer_home_domain: row.issuer_home_domain,
        contract_id: row.contract_id,
        sac_contract_id,
        sac_deployed,
        name: row.name,
        symbol: row.symbol,
        decimals: row.decimals,
        total_supply: row.total_supply,
        holder_count: row.holder_count,
        icon_url: row.icon_url,
    }
}

/// Forms of `:id` (the numeric surrogate was dropped — PR #175 / the composite
/// move). The first that parses cleanly drives the SQL. `native` is a reserved
/// token: the classic native XLM singleton (`asset_type = 0`) carries no
/// composite identity (no contract_id, no code/issuer per `ck_assets_identity`),
/// so it has no StrKey / CODE-ISSUER to address it by.
enum AssetIdRef<'a> {
    Native,
    Contract(&'a str),
    CodeIssuer(&'a str, &'a str),
}

fn parse_asset_id(raw: &str) -> Option<AssetIdRef<'_>> {
    if raw.eq_ignore_ascii_case("native") {
        return Some(AssetIdRef::Native);
    }
    if is_strkey_shape(raw, 'C') {
        return Some(AssetIdRef::Contract(raw));
    }
    // Codes never contain `-`; split on the LAST one and validate the
    // issuer half as a G-StrKey to disambiguate from C-StrKeys with stray dashes.
    if let Some(idx) = raw.rfind('-')
        && idx > 0
        && idx < raw.len() - 1
    {
        let code = &raw[..idx];
        let issuer = &raw[idx + 1..];
        if is_strkey_shape(issuer, 'G') {
            return Some(AssetIdRef::CodeIssuer(code, issuer));
        }
    }
    None
}

#[utoipa::path(
    get,
    path = "/assets",
    tag = "assets",
    params(
        ("limit" = Option<u32>, Query,
         description = "Items per page (1–100, default 20).",
         minimum = 1, maximum = 100),
        ("cursor" = Option<String>, Query,
         description = "Opaque pagination cursor from a previous response."),
        ListParams,
    ),
    responses(
        (status = 200, description = "Paginated asset list",
         body = Paginated<AssetItem>),
        (status = 400, description = "Invalid query parameter", body = ErrorEnvelope),
        (status = 500, description = "Internal server error",   body = ErrorEnvelope),
    ),
)]
pub async fn list_assets(
    State(state): State<AppState>,
    pagination: Pagination<AssetKeyCursor>,
    Query(params): Query<ListParams>,
) -> Response {
    let asset_type: Option<i16> = match filters::parse_enum_opt::<AssetFamily>(
        params.filter_type.as_deref(),
        "type",
        Some("asset type"),
    ) {
        Ok(maybe) => maybe.map(|t| t as i16),
        Err(resp) => return resp,
    };

    if let Err(resp) = filters::reject_sql_wildcards_opt(params.filter_code.as_deref(), "code") {
        return resp;
    }

    // SAC property filter (ADR 0051): `filter[sac]=true` → the SAC view.
    let sac_only = params
        .filter_sac
        .as_deref()
        .is_some_and(|v| v.eq_ignore_ascii_case("true") || v == "1");

    let direction = pagination.direction;
    let has_predecessor = pagination.has_predecessor();
    let resolved = ResolvedListParams {
        limit: pagination.fetch_limit(),
        cursor: pagination.cursor,
        asset_type,
        asset_code: params.filter_code,
        sac_only,
    };

    let mut rows: Vec<AssetRow> = match fetch_list_for_source(&state, &resolved, direction).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "DB error in list_assets");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    let page = finalize_page(
        &mut rows,
        pagination.limit,
        direction,
        has_predecessor,
        |dir, r| {
            cursor::encode(
                &AssetKeyCursor {
                    asset_type: r.asset_type,
                    asset_code: r.asset_code.clone().unwrap_or_default(),
                    issuer_id: r.issuer_id,
                    contract_id: r.contract_surrogate_id,
                },
                dir,
            )
        },
    );
    let data: Vec<AssetItem> = rows
        .into_iter()
        .map(|r| map_item(r, &state.network_id))
        .collect();

    let mut resp = Json(into_envelope(data, page)).into_response();
    cache_control::attach(&mut resp, cache_control::SHORT);
    resp
}

#[utoipa::path(
    get,
    path = "/assets/{id}",
    tag = "assets",
    params(
        ("id" = String, Path,
         description = "Contract StrKey (C…, 56 chars), `CODE-ISSUER` composite (e.g. USDC-GA…), or the reserved `native` token for XLM."),
    ),
    responses(
        (status = 200, description = "Asset detail", body = AssetDetailResponse),
        (status = 400, description = "Invalid id format", body = ErrorEnvelope),
        (status = 404, description = "Asset not found",   body = ErrorEnvelope),
        (status = 500, description = "Internal server error", body = ErrorEnvelope),
    ),
)]
pub async fn get_asset(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let parsed = match parse_asset_id(&id) {
        Some(p) => p,
        None => {
            return errors::bad_request_with_details(
                errors::INVALID_ID,
                "id must be a contract StrKey (C…, 56 chars), \
                 a `CODE-ISSUER` composite (e.g. USDC-GA…XYZ), \
                 or the reserved `native` token for XLM",
                serde_json::json!({ "received": id }),
            );
        }
    };

    let row = match fetch_asset_row_for_source(&state, parsed).await {
        Ok(Some(r)) => r,
        Ok(None) => return errors::not_found("asset not found"),
        Err(e) => {
            tracing::error!(asset_id = %id, error = %e, "DB error fetching asset");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    let deployed_at_ledger = row.deployed_at_ledger;
    let issuer = row.issuer.clone();
    let asset_code = row.asset_code.clone();
    let home_domain = row.issuer_home_domain.clone();

    // SEP-1 runtime enrichment (task 0188). `description` ← matched
    // `CURRENCIES[].desc`; `home_page` ← `DOCUMENTATION.ORG_URL`. Native
    // XLM / no-issuer Soroban tokens / accounts without `home_domain`
    // skip the fetch and surface both as `null`. A real fetch failure
    // also yields `null` (warn-logged) — the API never propagates a 5xx
    // because of an enrichment failure.
    let (description, home_page) = match home_domain.as_deref() {
        Some(domain) if !domain.is_empty() => {
            match state.runtime_enrichment.sep1.fetch(domain).await {
                Ok(parsed) => {
                    extract_sep1_fields(&parsed, asset_code.as_deref(), issuer.as_deref())
                }
                Err(e) => {
                    tracing::warn!(home_domain = %domain, error = %e, "SEP-1 fetch failed for issuer home_domain");
                    (None, None)
                }
            }
        }
        _ => (None, None),
    };

    let response = AssetDetailResponse {
        item: map_item(row, &state.network_id),
        deployed_at_ledger,
        description,
        home_page,
    };
    let mut resp = Json(response).into_response();
    cache_control::attach(&mut resp, cache_control::MEDIUM);
    resp
}

/// Pull the two SEP-1 fields the API exposes: `desc` from the matching
/// `CURRENCIES[]` row (by `code` + `issuer`) and `ORG_URL` from
/// `DOCUMENTATION` (used as `home_page` since SEP-1 has no per-currency
/// homepage field). A missing currency match still yields the org URL —
/// useful when the issuer publishes their site but doesn't list every
/// individual token.
fn extract_sep1_fields(
    parsed: &Sep1TomlParsed,
    asset_code: Option<&str>,
    issuer: Option<&str>,
) -> (Option<String>, Option<String>) {
    let description = match (asset_code, issuer) {
        (Some(code), Some(iss)) => {
            find_currency(&parsed.currencies, code, iss).and_then(|c| c.desc.clone())
        }
        _ => None,
    };
    let home_page = parsed
        .documentation
        .as_ref()
        .and_then(|d| d.org_url.clone());
    (description, home_page)
}

/// Find the `CURRENCIES[]` entry whose `code` and `issuer` match the
/// queried asset. Returns the first match (SEP-1 does not require codes
/// to be unique, but in practice they are per issuer).
fn find_currency<'a>(
    currencies: &'a [Sep1Currency],
    asset_code: &str,
    issuer: &str,
) -> Option<&'a Sep1Currency> {
    currencies
        .iter()
        .find(|c| c.code.as_deref() == Some(asset_code) && c.issuer.as_deref() == Some(issuer))
}

/// `:id/transactions` dispatch — the composed read (task 0359): the
/// `operation_asset_appearances` fan-out on `id` UNION the
/// `soroban_invocations_appearances` activity of the asset's contract
/// surrogate(s). The surrogates make EVERY asset type complete: a Soroban-native
/// (type-3) token's own contract (its fan-out is empty — the F2/#1 fix), and a
/// classic/native asset's SAC facet (its SAC-contract activity — F-F).
async fn fetch_asset_tx_for_source(
    state: &AppState,
    row: &AssetRow,
    limit: i64,
    cursor: Option<&TxListCursor>,
    direction: Direction,
) -> Result<Vec<AssetTxRow>, clickhouse::error::Error> {
    // ADR 0051: an asset has ONE associated contract, never both — its own
    // contract if it is a Soroban-native (type-3) token, else the wrapping SAC of
    // a classic / native asset. Arm B seeks that contract's invocations.
    let contract_surrogate = if row.contract_surrogate_id != 0 {
        Some(row.contract_surrogate_id)
    } else if row.sac_contract_surrogate != 0 {
        Some(row.sac_contract_surrogate)
    } else {
        None
    };
    queries::fetch_transactions(
        &state.ch(),
        row.id,
        contract_surrogate,
        limit,
        cursor,
        direction,
    )
    .await
}

/// Build the opaque asset-transactions cursor for a boundary row. CH keys on
/// `(ledger_sequence, id)`.
fn asset_tx_cursor_for(r: &AssetTxRow) -> TxListCursor {
    TxListCursor::Ch {
        ledger_sequence: r.ledger_sequence,
        tiebreak: r.id,
    }
}

/// True when the decoded cursor is a current (CH) cursor. A stale cursor minted
/// under the retired PG backend is rejected (ADR 0008 fail-clean).
fn cursor_matches_source(cursor: &TxListCursor) -> bool {
    matches!(cursor, TxListCursor::Ch { .. })
}

#[utoipa::path(
    get,
    path = "/assets/{id}/transactions",
    tag = "assets",
    params(
        ("id" = String, Path,
         description = "Contract StrKey (C…, 56 chars), `CODE-ISSUER` composite (e.g. USDC-GA…), or the reserved `native` token for XLM."),
        ("limit" = Option<u32>, Query,
         description = "Items per page (1–100, default 20).",
         minimum = 1, maximum = 100),
        ("cursor" = Option<String>, Query,
         description = "Opaque pagination cursor from a previous response."),
    ),
    responses(
        (status = 200, description = "Paginated transactions involving the asset",
         body = Paginated<AssetTransactionItem>),
        (status = 400, description = "Invalid id format / pagination", body = ErrorEnvelope),
        (status = 404, description = "Asset not found", body = ErrorEnvelope),
        (status = 500, description = "Internal server error", body = ErrorEnvelope),
    ),
)]
pub async fn list_asset_transactions(
    State(state): State<AppState>,
    pagination: Pagination<TxListCursor>,
    Path(id): Path<String>,
) -> Response {
    let parsed = match parse_asset_id(&id) {
        Some(p) => p,
        None => {
            return errors::bad_request_with_details(
                errors::INVALID_ID,
                "id must be a contract StrKey (C…, 56 chars), \
                 a `CODE-ISSUER` composite (e.g. USDC-GA…XYZ), \
                 or the reserved `native` token for XLM",
                serde_json::json!({ "received": id }),
            );
        }
    };

    // Reject a stale cursor minted under the retired PG backend — its keyset is
    // meaningless under CH (ADR 0008 fail-clean). A legacy/untagged cursor
    // already fails decode upstream; this guards the decodes-but-wrong-intent
    // case.
    if let Some(cursor) = &pagination.cursor
        && !cursor_matches_source(cursor)
    {
        return errors::bad_request(errors::INVALID_CURSOR, "cursor is malformed or expired");
    }

    let row = match fetch_asset_row_for_source(&state, parsed).await {
        Ok(Some(r)) => r,
        Ok(None) => return errors::not_found("asset not found"),
        Err(e) => {
            tracing::error!(asset_id = %id, error = %e, "DB error fetching asset");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    // ADR 0051 / task 0339: discriminant 2 (retired `sac`) no longer exists in
    // prod (Phase-2 relabel complete) and is rejected by `try_from` like any
    // other unknown discriminant.
    if AssetFamily::try_from(row.asset_type).is_err() {
        tracing::error!(
            asset_type = row.asset_type,
            contract_id = ?row.contract_id,
            asset_code = ?row.asset_code,
            "unknown asset_type discriminant for asset"
        );
        return errors::internal_error(
            errors::DB_ERROR,
            "asset row carries an unknown asset_type discriminant",
        );
    }

    // Guard the unresolved-asset sentinel: a resolved asset always carries a real
    // `ids::asset_id` surrogate, so `id == 0` means "no such asset key" — return an
    // empty page rather than run `WHERE asset_id = 0`. Native is NOT caught here:
    // it has a first-class surrogate (`ids::asset_id(0,"",0,0)`, non-zero), so the
    // fan-out seek runs and native activity is served (task 0359 / devils-advocate
    // C6, PR #9 — this replaces the old identity gate that emptied native because
    // it has no classic code/issuer/contract).
    if row.id == 0 {
        let empty = into_envelope::<AssetTransactionItem>(
            Vec::new(),
            PageInfo {
                next_cursor: None,
                prev_cursor: None,
                limit: pagination.limit,
            },
        );
        let mut resp = Json(empty).into_response();
        cache_control::attach(&mut resp, cache_control::SHORT);
        return resp;
    }

    let mut rows = match fetch_asset_tx_for_source(
        &state,
        &row,
        pagination.fetch_limit(),
        pagination.cursor.as_ref(),
        pagination.direction,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(asset_id = %id, error = %e, "DB error in list_asset_transactions");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    let page = finalize_page(
        &mut rows,
        pagination.limit,
        pagination.direction,
        pagination.has_predecessor(),
        |dir, r| cursor::encode(&asset_tx_cursor_for(r), dir),
    );
    let data: Vec<AssetTransactionItem> = rows
        .into_iter()
        .map(|r| AssetTransactionItem {
            hash: r.hash,
            ledger_sequence: r.ledger_sequence,
            source_account: r.source_account,
            successful: r.successful,
            fee_charged: r.fee_charged,
            created_at: r.created_at,
            operation_count: r.operation_count,
            has_soroban: r.has_soroban,
            operation_types: r.operation_types,
        })
        .collect();

    let mut resp = Json(into_envelope(data, page)).into_response();
    cache_control::attach(&mut resp, cache_control::SHORT);
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    // Shape-valid StrKeys (56 chars, correct prefix, base32) — never minted on
    // mainnet, used purely to exercise the parser's prefix/shape branches.
    const C_STRKEY: &str = "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAJ";
    const G_STRKEY: &str = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAT";

    fn asset_row(
        asset_type: i16,
        asset_code: Option<&str>,
        issuer: Option<&str>,
        contract_id: Option<&str>,
    ) -> AssetRow {
        AssetRow {
            asset_type,
            asset_type_name: None,
            asset_code: asset_code.map(String::from),
            issuer: issuer.map(String::from),
            contract_id: contract_id.map(String::from),
            name: None,
            symbol: None,
            decimals: 7,
            total_supply: None,
            holder_count: None,
            icon_url: None,
            deployed_at_ledger: None,
            issuer_home_domain: None,
            issuer_id: 0,
            contract_surrogate_id: 0,
            sac_contract_surrogate: 0,
            sac_deployed: false,
            id: 0,
        }
    }

    fn parsed_label(parsed: &Option<AssetIdRef<'_>>) -> &'static str {
        match parsed {
            None => "None",
            Some(AssetIdRef::Native) => "Native",
            Some(AssetIdRef::Contract(_)) => "Contract",
            Some(AssetIdRef::CodeIssuer(..)) => "CodeIssuer",
        }
    }

    // -- parse_asset_id ------------------------------------------------------

    #[test]
    fn parse_native_token_case_insensitive() {
        for raw in ["native", "NATIVE", "Native", "nAtIvE"] {
            assert!(
                matches!(parse_asset_id(raw), Some(AssetIdRef::Native)),
                "{raw:?} should parse as Native"
            );
        }
    }

    #[test]
    fn parse_contract_strkey() {
        assert!(matches!(
            parse_asset_id(C_STRKEY),
            Some(AssetIdRef::Contract(c)) if c == C_STRKEY
        ));
    }

    #[test]
    fn parse_code_issuer_composite() {
        let raw = format!("USDC-{G_STRKEY}");
        match parse_asset_id(&raw) {
            Some(AssetIdRef::CodeIssuer(code, issuer)) => {
                assert_eq!(code, "USDC");
                assert_eq!(issuer, G_STRKEY);
            }
            other => panic!("expected CodeIssuer, got {}", parsed_label(&other)),
        }
    }

    #[test]
    fn parse_code_issuer_splits_on_last_dash() {
        // A code is never supposed to contain `-`, but if a hyphenated string
        // arrives the split is on the LAST dash so the issuer half validates.
        let raw = format!("WEIRD-CODE-{G_STRKEY}");
        match parse_asset_id(&raw) {
            Some(AssetIdRef::CodeIssuer(code, issuer)) => {
                assert_eq!(code, "WEIRD-CODE");
                assert_eq!(issuer, G_STRKEY);
            }
            other => panic!("expected CodeIssuer, got {}", parsed_label(&other)),
        }
    }

    #[test]
    fn parse_rejects_numeric_id() {
        // The dropped numeric surrogate — must NOT resolve (handler → 400).
        for raw in ["12345", "0", "2147483647", "-1"] {
            assert!(
                parse_asset_id(raw).is_none(),
                "{raw:?} (numeric) must not parse"
            );
        }
    }

    #[test]
    fn parse_rejects_malformed() {
        for raw in [
            "",                     // empty
            "not-an-asset-id",      // dash, but `id` is not a G-StrKey
            "USDC-",                // trailing dash → issuer empty
            "-GAAA",                // leading dash → idx == 0
            G_STRKEY,               // bare G-StrKey (not C, no dash)
            "USDC-NOTAVALIDISSUER", // issuer half wrong shape
            "CAAA",                 // too-short C prefix, no dash
            "nativex",              // close-but-not `native`, no dash, not C
        ] {
            assert!(parse_asset_id(raw).is_none(), "{raw:?} must not parse");
        }
    }

    #[test]
    fn parse_code_issuer_requires_g_prefix_issuer() {
        // A C-StrKey in the issuer position is rejected (only G addresses issue).
        let raw = format!("USDC-{C_STRKEY}");
        assert!(parse_asset_id(&raw).is_none());
    }

    // -- canonical_id (wire token) ------------------------------------------

    #[test]
    fn canonical_id_prefers_contract_strkey() {
        // Defensive precedence: a present key `contract_id` wins over CODE-ISSUER.
        // Post-ADR 0051 a row never carries both (soroban has only a contract id,
        // classic only code+issuer), but the ordering is still the contract.
        let row = asset_row(3, Some("USDC"), Some(G_STRKEY), Some(C_STRKEY));
        assert_eq!(canonical_id(&row), C_STRKEY);
    }

    #[test]
    fn map_item_rederives_sac_strkey_for_classic_wrap() {
        // ADR 0051: a classic_credit asset with an observed SAC surfaces the
        // re-derived C… StrKey (never stored) + the deployed flag. USDC's mainnet
        // SAC is a published constant — regression-guards the read-side derivation.
        const USDC_ISSUER: &str = "GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN";
        let net = xdr_parser::network_id(xdr_parser::MAINNET_PASSPHRASE);

        let mut row = asset_row(1, Some("USDC"), Some(USDC_ISSUER), None);
        row.sac_contract_surrogate = 42; // non-zero ⇒ "has SAC" (value irrelevant)
        row.sac_deployed = true;
        let item = map_item(row, &net);
        assert_eq!(
            item.sac_contract_id.as_deref(),
            Some("CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75")
        );
        assert_eq!(item.sac_deployed, Some(true));
        assert_eq!(item.contract_id, None); // key contract_id stays soroban-only

        // No observed SAC ⇒ no facet on the wire.
        let plain = asset_row(1, Some("USDC"), Some(USDC_ISSUER), None);
        let plain_item = map_item(plain, &net);
        assert_eq!(plain_item.sac_contract_id, None);
        assert_eq!(plain_item.sac_deployed, None);
    }

    #[test]
    fn canonical_id_contract_only() {
        let row = asset_row(3, None, None, Some(C_STRKEY));
        assert_eq!(canonical_id(&row), C_STRKEY);
    }

    #[test]
    fn canonical_id_code_issuer_for_classic() {
        let row = asset_row(1, Some("USDC"), Some(G_STRKEY), None);
        assert_eq!(canonical_id(&row), format!("USDC-{G_STRKEY}"));
    }

    #[test]
    fn canonical_id_native_singleton() {
        let row = asset_row(0, None, None, None);
        assert_eq!(canonical_id(&row), "native");
    }

    #[test]
    fn canonical_id_empty_fallback_is_unreachable_but_safe() {
        // Defensive last arm: a non-native asset with neither a contract id nor
        // a full code+issuer pair violates `ck_assets_identity` (unreachable in
        // real data), but the token builder degrades to "" rather than panic.
        let code_only = asset_row(1, Some("USDC"), None, None);
        assert_eq!(canonical_id(&code_only), "");
        let issuer_only = asset_row(1, None, Some(G_STRKEY), None);
        assert_eq!(canonical_id(&issuer_only), "");
    }

    #[test]
    fn canonical_id_roundtrips_through_parse() {
        // Every token canonical_id emits must parse back to the same form.
        let contract = asset_row(2, None, None, Some(C_STRKEY));
        assert!(matches!(
            parse_asset_id(&canonical_id(&contract)),
            Some(AssetIdRef::Contract(_))
        ));

        let classic = asset_row(1, Some("USDC"), Some(G_STRKEY), None);
        assert!(matches!(
            parse_asset_id(&canonical_id(&classic)),
            Some(AssetIdRef::CodeIssuer(..))
        ));

        let native = asset_row(0, None, None, None);
        assert!(matches!(
            parse_asset_id(&canonical_id(&native)),
            Some(AssetIdRef::Native)
        ));
    }
}
