//! Library surface for the `api` crate.
//!
//! Auxiliary binaries (e.g. `extract_openapi`) consume only
//! [`openapi::register_routes`]; the rest of the crate's modules are
//! crate-private to keep the public surface narrow.
//!
//! `dead_code` is silenced at crate level because the binary target
//! (`main.rs`) compiles its own copy of these modules and exercises
//! the items the lib does not reach (e.g. `AppConfig::from_env`,
//! `default_timeout_config`). Keeping them in the lib lets
//! `register_routes` reference shared types without forcing the bin
//! to import twice.
#![allow(dead_code)]

mod assets;
mod cache;
mod common;
mod config;
mod contracts;
mod ledgers;
mod liquidity_pools;
mod network;
pub mod openapi;
mod ops;
mod search;
mod state;
mod stellar_archive;
mod transactions;

pub use state::AppState;
