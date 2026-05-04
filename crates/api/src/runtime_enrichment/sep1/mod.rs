//! HTTP fetch of issuer stellar.toml files (SEP-1) for runtime details enrichment.
//!
//! Per the M2 enrichment plan, this submodule will provide a fail-soft, LRU-cached
//! reqwest-based client that resolves an issuer's `home_domain` to its stellar.toml
//! and parses the SEP-1 fields (`CURRENCIES[]`, `DOCUMENTATION`, etc.) consumed by
//! `GET /v1/assets/{id}` and (once accounts ships) `GET /v1/accounts/{id}`.
//!
//! Skeleton intentionally empty — body is wired in the follow-up task spawned
//! after this refactor lands.

// TODO(0187 follow-up): impl SEP-1 stellar.toml fetcher.
