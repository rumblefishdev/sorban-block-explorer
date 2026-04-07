//! PostgreSQL connection pool configuration for Lambda.
//!
//! Lambda functions should use `max_connections(1)` with RDS Proxy.

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

/// Create a PgPool configured for Lambda (single connection, RDS Proxy).
///
/// Uses `connect_lazy` to avoid opening a DB connection during cold start.
pub fn create_pool(database_url: &str) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(1)
        .connect_lazy(database_url)
}
