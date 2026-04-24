//! Shared application state injected into every axum handler via `State<AppState>`.

use sqlx::PgPool;

use crate::stellar_archive::StellarArchiveFetcher;

/// Application-wide state. Both inner types are cheaply cloneable (`Arc`-backed).
#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub fetcher: StellarArchiveFetcher,
}
