//! Shared HTTP-based enrichment building blocks.
//!
//! Hosts the SEP-1 stellar.toml fetcher (moved here from
//! `api::runtime_enrichment::sep1` per task 0191) so it can be reused by:
//!
//! - the api crate's [`runtime_enrichment`] module (per-request type-2),
//! - the `enrichment-worker` Lambda (SQS-driven type-1),
//! - any future local backfill / refresh tool that needs the same fetch
//!   surface (no api dep, no cyclic crate graph).
//!
//! Add a new submodule per external source as it is needed (e.g. an LP
//! price oracle for task 0125). Keep cross-cutting concerns (HTTP client
//! construction, error mapping, in-process cache wiring) inside the
//! relevant submodule rather than promoting them to lib root — each
//! source has its own caching / timeout / SSRF profile and the modules
//! should not leak details across each other.
//!
//! [`runtime_enrichment`]: ../api/runtime_enrichment/index.html

pub mod enrich_and_persist;
pub mod sep1;
