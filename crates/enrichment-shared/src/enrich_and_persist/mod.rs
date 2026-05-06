//! Per-kind enrichment functions — the unit of work invoked by both the
//! type-1 worker Lambda (one call per SQS message) and any future local
//! backfill / refresh tool (one call per row pulled from a streaming
//! SELECT).
//!
//! Each `enrich_*` function owns the full "fetch externally + write the
//! target column(s)" path for a single row. Worker / backfill code only
//! has to drive iteration — they don't reimplement HTTP, parsing, or DB
//! writes.

pub mod error;
pub mod icon;

pub use error::EnrichError;
