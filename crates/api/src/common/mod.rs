//! Cross-endpoint helpers shared by every collection module under `/v1/…`.
//!
//! Organised as narrow, composable submodules so a handler can pick just
//! the pieces it needs. Resources compose these primitives directly;
//! there is no `CrudResource` trait layer — see the post-audit note in
//! archived task 0043 for why it was built, audited, and removed.
//!
//! See task 0043 and ADR 0008.

pub mod asset_identity;
pub mod cache_control;
pub mod ch;
pub mod conditional;
pub mod cursor;
pub mod edge_lock;
pub mod errors;
pub mod extractors;
pub mod filters;
pub mod head;
pub mod pagination;
pub mod path;
pub mod pool_asset_codes;
pub mod request_id;
pub mod strkey;
