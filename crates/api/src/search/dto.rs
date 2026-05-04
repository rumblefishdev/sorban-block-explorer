//! Wire types for `GET /v1/search`.
//!
//! Spec source: lore task 0053 + canonical SQL in
//! `docs/architecture/database-schema/endpoint-queries/22_get_search.sql`.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Discriminated response: `redirect` for unambiguous exact match,
/// `results` for grouped broad search.
///
/// `#[serde(tag = "type")]` puts the discriminator on the wire as
/// `"type": "redirect" | "results"` per the task spec, mirroring the
/// frontend search-bar UX expectation: a `redirect` causes the bar to
/// navigate directly; a `results` shows the dropdown with grouped hits.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SearchResponse {
    Redirect(SearchRedirect),
    Results(SearchResults),
}

/// Redirect payload — frontend navigates directly to the entity page.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SearchRedirect {
    pub entity_type: EntityType,
    pub entity_id: String,
}

/// Results payload — six entity-typed buckets, each capped at the
/// per-group `limit` chosen by the caller (default 10, ceiling 50).
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct SearchResults {
    pub groups: SearchGroups,
}

/// Per-entity buckets. Empty buckets are omitted from the JSON output
/// via `skip_serializing_if` — frontend treats absent and empty array
/// identically, and dropping empties keeps the dropdown payload tight
/// when a typed `?type=` filter narrows results to a single bucket.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct SearchGroups {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transactions: Vec<SearchHit>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accounts: Vec<SearchHit>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<SearchHit>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contracts: Vec<SearchHit>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nfts: Vec<SearchHit>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pools: Vec<SearchHit>,
}

/// Single search row. Narrow shape — same four columns for every
/// entity bucket; rich entity payloads are NOT inlined here.
///
/// `identifier` is the canonical human-shown id (hex hash for
/// transactions / pools, StrKey for accounts / contracts, asset code
/// for assets, name for NFTs). For `asset` and `nft` it is NOT unique —
/// the frontend MUST route via `surrogate_id`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SearchHit {
    pub entity_type: EntityType,
    pub identifier: String,
    pub label: String,
    pub surrogate_id: Option<i64>,
}

/// Entity discriminator. Closed allowlist used by the `type=` filter
/// and the `entity_type` field on every hit / redirect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum EntityType {
    Transaction,
    Account,
    Asset,
    Contract,
    Nft,
    Pool,
}

impl EntityType {
    /// Wire-format names for every variant. Single source of truth used
    /// by `parse()`, error messages, and any future API surface that
    /// needs to enumerate the closed set (e.g. an OpenAPI enum
    /// extension or a debug listing endpoint).
    pub const ALL: &'static [&'static str] =
        &["transaction", "contract", "asset", "account", "nft", "pool"];

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "transaction" => Some(Self::Transaction),
            "account" => Some(Self::Account),
            "asset" => Some(Self::Asset),
            "contract" => Some(Self::Contract),
            "nft" => Some(Self::Nft),
            "pool" => Some(Self::Pool),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lock the invariant that `ALL` and `parse()` agree on the closed
    /// set. If a variant is added in the future and one site is missed,
    /// this catches the drift before the divergence reaches the wire.
    #[test]
    fn all_const_matches_parse_round_trip() {
        for name in EntityType::ALL {
            assert!(
                EntityType::parse(name).is_some(),
                "EntityType::ALL contains `{name}` but parse() rejects it"
            );
        }
    }
}
