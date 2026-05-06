//! HTTP fetch of issuer stellar.toml files (SEP-1).
//!
//! Per-request, fail-soft, in-process LRU-cached. Resolves an issuer's on-chain
//! `home_domain` to `https://{home_domain}/.well-known/stellar.toml`, parses the
//! TOML, and surfaces the relevant slice for downstream consumers.
//!
//! Originated in `api::runtime_enrichment::sep1` for task 0188 (per-request
//! type-2 detail enrichment); moved here in task 0191 so the worker Lambda
//! and a future backfill tool can share the same fetcher without a cyclic
//! dependency on the api crate.
//!
//! Reference: <https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0001.md>

mod client;
pub mod dto;
pub mod errors;

pub use client::Sep1Fetcher;
pub use dto::{Sep1Currency, Sep1TomlParsed};
// `Sep1Documentation` and `Sep1Error` are intentionally accessed by their
// fully-qualified path (`sep1::dto::Sep1Documentation`, `sep1::errors::Sep1Error`)
// since no current consumer needs them re-exported. Add re-exports here when
// the second consumer materialises.
