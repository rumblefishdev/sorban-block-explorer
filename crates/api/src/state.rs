//! Shared application state injected into every axum handler via `State<AppState>`.

use sqlx::PgPool;

use crate::contracts::cache::ContractMetadataCache;
use crate::network::cache::NetworkStatsCache;
use crate::runtime_enrichment::RuntimeEnrichment;

/// Application-wide state. All inner types are cheaply cloneable
/// (`Arc`-backed; both `moka::sync::Cache` and `moka::future::Cache`
/// clones are refcount bumps; `reqwest::Client` is also `Arc`-backed).
#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    /// Bundle of runtime-enrichment fetchers (S3 stellar archive + SEP-1
    /// stellar.toml). One struct so the field count on `AppState` doesn't
    /// grow per new transport, and so the grouping mirrors the
    /// `runtime_enrichment` module structure 1:1.
    pub runtime_enrichment: RuntimeEnrichment,
    /// Per-Lambda warm cache for contract detail responses (45 s TTL).
    pub contract_cache: ContractMetadataCache,
    /// Per-Lambda warm cache for the `/v1/network/stats` singleton (30 s TTL).
    pub network_cache: NetworkStatsCache,
    /// `SHA256(STELLAR_NETWORK_PASSPHRASE)`. Required to align tx_set
    /// envelopes (hash-sorted) with `tx_processing` (apply order) when
    /// re-extracting heavy fields from archive XDR.
    pub network_id: [u8; 32],
}
