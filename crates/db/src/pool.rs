//! PostgreSQL connection pool configuration.
//!
//! Entry point for creating a database connection pool. Default configuration
//! uses a single connection, optimized for Lambda callers fronted by RDS Proxy.

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
