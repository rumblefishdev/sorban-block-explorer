//! Runtime details enrichment — per-request, fail-soft, in-process.
//!
//! Two transport-specific submodules share a common shape: best-effort fetch,
//! merge into the DB-light slice, signal status via an `enrichment_status`
//! field on the response.
//!
//! - [`stellar_archive`] — S3 reread of public Stellar archive ledgers (ADR 0029).
//! - [`sep1`] — HTTP fetch of issuer stellar.toml files (M2 follow-up; skeleton only).

pub mod sep1;
pub mod stellar_archive;
