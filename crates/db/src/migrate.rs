//! Compile-time migration embedding via `sqlx::migrate!()`.

use sqlx::PgPool;

/// Run all pending migrations against the given pool.
///
/// Migrations are embedded at compile time from `crates/db/migrations/`.
/// Already-applied migrations are skipped (idempotent).
pub async fn run_migrations(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("./migrations").run(pool).await
}
