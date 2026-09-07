//! Request and response DTOs for the assets endpoints.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// `filter[...]` query parameters for `GET /v1/assets`.
///
/// `limit` / `cursor` are read by a sibling `Pagination<AssetIdCursor>`
/// extractor and are intentionally absent here.
#[derive(Debug, Deserialize, IntoParams)]
pub struct ListParams {
    /// `native | classic_credit | soroban` (ADR 0051 — `sac` is no longer a
    /// type; use `filter[sac]` for the SAC view).
    #[serde(rename = "filter[type]")]
    pub filter_type: Option<String>,
    /// Substring match against `asset_code`; SQL wraps in `%...%`.
    /// Caller MUST NOT pass `%` / `_` literals.
    #[serde(rename = "filter[code]")]
    pub filter_code: Option<String>,
    /// SAC property filter (ADR 0051): `true` restricts the list to assets whose
    /// Stellar Asset Contract is deployed on-chain (`sac_deployed`) — the old
    /// `filter[type]=sac` view, now a facet predicate over classic_credit /
    /// native rows. Reserved (un-deployed) SAC addresses are excluded. Any other
    /// value is ignored.
    #[serde(rename = "filter[sac]")]
    pub filter_sac: Option<String>,
}

/// Asset row returned by list and detail. Surfaces both the decoded
/// `asset_type_name` (SQL `asset_family_name()`) and the raw `asset_type`
/// SMALLINT — canonical SQL `08_get_assets_list.sql` projection.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AssetItem {
    /// Canonical identifier — the single token usable as `/assets/{id}`:
    /// the reserved `native` token for native XLM, the contract StrKey
    /// (`C…`) for contract-backed assets (SAC / Soroban), otherwise the
    /// `CODE-ISSUER` composite (classic credit, e.g. `USDC-GA…`). Replaces
    /// the dropped numeric surrogate (PR #175): assets are keyed on
    /// `(asset_type, asset_code, issuer_id, contract_id)`, with no surrogate.
    pub id: String,
    /// `native | classic_credit | soroban` (ADR 0051 — `sac` retired). `null`
    /// only on schema drift.
    pub asset_type_name: Option<String>,
    /// Raw SMALLINT (0=native, 1=classic_credit, 3=soroban). 2 (`sac`) is retired.
    pub asset_type: i16,
    pub asset_code: Option<String>,
    pub issuer: Option<String>,
    /// The issuer account's on-chain `home_domain` (SEP-1 anchor domain), e.g.
    /// `centre.io`. Read from the same `accounts` key-seek that resolves
    /// `issuer`, so it costs no extra query (task 0450). `null` for native /
    /// Soroban-native assets (no issuer) and for issuers that never set one —
    /// most do not.
    ///
    /// NOT a verified identity: the account holder sets this field itself and
    /// nothing checks it. Render it as a claim, never as a badge implying we
    /// confirmed the domain owns the asset.
    ///
    /// Distinct from the detail response's `home_page`, which is fetched from
    /// the issuer's `stellar.toml` at request time.
    pub issuer_home_domain: Option<String>,
    /// Soroban contract StrKey (`C…`) — set ONLY for `soroban` (type=3), where
    /// the contract IS the asset. `null` for native / classic_credit (a wrapping
    /// SAC's address rides in `sac_contract_id`).
    pub contract_id: Option<String>,
    /// SAC facet (ADR 0051): the wrapping Stellar Asset Contract's `C…` StrKey
    /// for a native / classic_credit asset that has one, re-derived on read from
    /// `code:issuer` (never stored). `null` when the asset has no observed SAC.
    pub sac_contract_id: Option<String>,
    /// Whether `sac_contract_id`'s SAC is deployed on-chain (ADR 0051). `null`
    /// when the asset has no SAC; `false` = reserved-but-un-deployed address
    /// (render the contract link non-clickable).
    pub sac_deployed: Option<bool>,
    pub name: Option<String>,
    /// On-chain SEP-41 token symbol (Soroban `METADATA`). `null` for classic
    /// (use `asset_code`) and native.
    pub symbol: Option<String>,
    /// Display decimals — on-chain `METADATA` for Soroban tokens, else 7
    /// (Stellar classic precision). Load-bearing for amount rendering.
    pub decimals: u32,
    /// Total supply as a RAW integer string (`Int128`) — scale by `decimals` for
    /// display (task 0331 Option C: one convention for ALL asset types; classic
    /// `decimals` is 7). E.g. `"63836094715548"`. `null` = no balance data
    /// (a token/asset with no holders). Sourced from `balance_aggregates` over the
    /// unified `balances` table.
    pub total_supply: Option<String>,
    /// Active-holder count (`amount > 0`) from the unified `balances` aggregate
    /// (all asset types — trustline holders + contract holders). `null` = no data.
    pub holder_count: Option<i32>,
    pub icon_url: Option<String>,
}

/// Detail response. `description` is populated from the issuer stellar.toml
/// `CURRENCIES[].desc` field; `home_page` is populated from
/// `DOCUMENTATION.ORG_URL` (SEP-1 has no per-currency `home_page` field —
/// the org URL is the closest semantic match and preserves backward
/// compatibility with the previous DB-sourced column). Both default to
/// `null` for native XLM, assets without an issuer, issuers without an
/// on-chain `home_domain`, fetch failures, and stellar.toml files with
/// no matching `[[CURRENCIES]]` entry.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AssetDetailResponse {
    #[serde(flatten)]
    #[schema(inline)]
    pub item: AssetItem,
    /// `soroban_contracts.deployed_at_ledger` — `null` for classic / native.
    pub deployed_at_ledger: Option<i64>,
    pub description: Option<String>,
    pub home_page: Option<String>,
}

/// Transaction row for `/assets/:id/transactions`. Pure-DB; mirrors
/// canonical SQL `10_get_assets_transactions.sql`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AssetTransactionItem {
    pub hash: String,
    pub ledger_sequence: i64,
    pub source_account: String,
    pub successful: bool,
    /// Fee charged, in raw stroops. Native (XLM) is always 7 decimals, so
    /// there is no `decimals` field — the frontend scales by 1e7.
    pub fee_charged: i64,
    pub created_at: DateTime<Utc>,
    pub operation_count: i16,
    pub has_soroban: bool,
    /// Distinct `op_type_name(...)` labels for every op in the tx, sorted asc.
    pub operation_types: Vec<String>,
}
/// Pagination payload for `GET /v1/assets`. The keyset walks the natural
/// identity 4-tuple `(asset_type, asset_code, issuer_id, contract_id)` — the
/// exact `ORDER BY` of the CH `assets` table. Serialized into the opaque wire
/// cursor (ADR 0008), so it lives on the DTO boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetKeyCursor {
    pub asset_type: i16,
    pub asset_code: String,
    pub issuer_id: i64,
    pub contract_id: i64,
}
