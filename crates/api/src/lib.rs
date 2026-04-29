//! Library surface for the `api` crate.
//!
//! This allows auxiliary binaries (for example OpenAPI extractors) to reuse
//! the same route modules and schema declarations as the Lambda entrypoint.

pub mod assets;
pub mod common;
pub mod config;
pub mod contracts;
pub mod liquidity_pools;
pub mod network;
pub mod openapi;
pub mod ops;
pub mod state;
pub mod stellar_archive;
pub mod transactions;
