//! Shared domain types for the Soroban block explorer.
//!
//! Read-path entity models aligned with ADR 0027 Part I — the post-surrogate
//! schema snapshot (surrogate `accounts.id`, BYTEA hashes, typed token
//! metadata, partitioned time-series tables).
//!
//! Write-path / ingestion types live in `xdr-parser::types::Extracted*`.

pub mod account;
pub mod balance;
pub mod ledger;
pub mod nft;
pub mod operation;
pub mod pool;
pub mod soroban;
pub mod token;
pub mod transaction;
