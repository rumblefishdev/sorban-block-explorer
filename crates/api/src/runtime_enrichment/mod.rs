//! Runtime details enrichment — per-request, fail-soft, in-process.
//!
//! Two transport-specific submodules share a common shape: best-effort fetch,
//! merge into the DB-light slice, signal status via an `enrichment_status`
//! field on the response.
//!
//! - [`stellar_archive`] — S3 reread of public Stellar archive ledgers (ADR 0029).
//! - [`sep1`] — HTTP fetch of issuer stellar.toml files (task 0188).

pub mod sep1;
pub mod stellar_archive;

use sep1::Sep1Fetcher;
use stellar_archive::StellarArchiveFetcher;

/// Bundle of every runtime-enrichment fetcher held in `AppState`.
///
/// Both inner fetchers are cheaply cloneable (`Arc`-backed). One field per
/// transport: a future submodule (e.g. `nft_metadata`, `price_oracle`)
/// would be added here without touching the existing surface.
#[derive(Clone)]
pub struct RuntimeEnrichment {
    pub stellar_archive: StellarArchiveFetcher,
    pub sep1: Sep1Fetcher,
}
