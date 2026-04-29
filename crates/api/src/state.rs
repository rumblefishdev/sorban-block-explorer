//! Shared application state injected into every axum handler via `State<AppState>`.

use sqlx::PgPool;

use crate::contracts::cache::ContractMetadataCache;
use crate::network::cache::NetworkStatsCache;
use crate::stellar_archive::StellarArchiveFetcher;

/// Application-wide state. All inner types are cheaply cloneable
/// (`Arc`-backed; `moka::sync::Cache` clones are refcount bumps).
#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub fetcher: StellarArchiveFetcher,
    /// Per-Lambda warm cache for contract detail responses (45 s TTL).
    pub contract_cache: ContractMetadataCache,
    /// Per-Lambda warm cache for the `/v1/network/stats` singleton (30 s TTL).
    pub network_cache: NetworkStatsCache,
    /// `SHA256(STELLAR_NETWORK_PASSPHRASE)`. Required to align tx_set
    /// envelopes (hash-sorted) with `tx_processing` (apply order) when
    /// re-extracting heavy fields from archive XDR.
    pub network_id: [u8; 32],
}
