//! HTTP fetch of issuer stellar.toml files (SEP-1) for runtime details enrichment.
//!
//! Per-request, fail-soft, in-process LRU-cached. Resolves an issuer's on-chain
//! `home_domain` to `https://{home_domain}/.well-known/stellar.toml`, parses the
//! TOML and surfaces the relevant slice on detail endpoints. Sibling to
//! [`super::stellar_archive`] — they share the architectural shape established
//! by ADR 0029 (best-effort fetch, merge into DB-light slice).
//!
//! Wired by task 0188 (M2 enrichment phase); first and currently only consumer
//! is `GET /v1/assets/{id}`. NFT and LP details have separate enrichment paths
//! (metadata-URI fetch and price oracle respectively) — they will not reuse
//! this fetcher.
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
