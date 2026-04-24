//! Cross-endpoint helpers shared by every collection module under `/v1/…`.
//!
//! Organised as narrow, composable submodules so a handler can pick just
//! the pieces it needs. The [`crud`] trait is an opt-in convenience layer
//! on top of these primitives for resources that have no custom
//! post-fetch enrichment; resources with bespoke needs (XDR fetch, StrKey
//! validation, join-dependent filters) use the lower-level modules directly.
//!
//! See task 0043 and ADR 0008.

pub mod crud;
pub mod cursor;
pub mod errors;
pub mod extractors;
pub mod filters;
pub mod pagination;
