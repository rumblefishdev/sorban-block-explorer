//! Database access layer for the Soroban block explorer.
//!
//! Provides sqlx PgPool configuration, migration embedding, and query functions.

pub mod migrate;
pub mod pool;

#[cfg(feature = "aws-secrets")]
pub mod secrets;
