//! Cross-endpoint helpers shared by every collection module under `/v1/…`.
//!
//! Organised as narrow, composable submodules so a handler can pick just
//! the pieces it needs. Resources compose these primitives directly;
//! there is no `CrudResource` trait layer (deferred — see task 0166 and
//! the post-audit note in archived task 0043).
//!
//! See task 0043 and ADR 0008.

pub mod cursor;
pub mod errors;
pub mod extractors;
pub mod filters;
pub mod pagination;
