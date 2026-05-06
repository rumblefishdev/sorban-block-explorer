//! Thin re-export shim over [`enrichment_shared::sep1`].
//!
//! The fetcher itself lives in the shared `enrichment-shared` crate so it
//! can be used by both the api crate's runtime (per-request type-2
//! enrichment) and the worker Lambda / future backfill tools (type-1).
//! Per task 0191; the move was a 1:1 lift from this module.
//!
//! Existing api-internal call sites import from
//! `crate::runtime_enrichment::sep1::…`; this shim keeps those imports
//! working without forcing every consumer to switch to the
//! `enrichment_shared` path. New external consumers (worker Lambda,
//! backfill tool) should import from `enrichment_shared::sep1` directly.

pub use enrichment_shared::sep1::{Sep1Currency, Sep1Fetcher, Sep1TomlParsed};
