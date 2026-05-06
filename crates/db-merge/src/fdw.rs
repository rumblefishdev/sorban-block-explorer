//! Set up and tear down the postgres_fdw bridge from the merge target
//! to the snapshot-source container. Per task 0186 §Step 2.2.
//!
//! After `setup`, the merge target sees every snapshot table as
//! `merge_source.<table>` — read-only, but supports JOINs against
//! local tables, which is exactly what the merge SQL needs.

use sqlx::{Executor, PgConnection};

use crate::error::MergeError;

/// FDW host/port/dbname as seen from inside the merge target container.
/// Hardcoded to the docker-compose service name so the postgres_fdw
/// connection resolves over the compose network. If the topology
/// changes, update both this constant and `docker-compose.yml`.
pub const FDW_HOST: &str = "postgres-snapshot-source";
pub const FDW_PORT: &str = "5432";
pub const FDW_DBNAME: &str = "soroban_block_explorer";
pub const FDW_USER: &str = "postgres";
pub const FDW_PASSWORD: &str = "postgres";

pub const FOREIGN_SCHEMA: &str = "merge_source";

/// Idempotent — safe to re-run after partial failure. Drops and
/// recreates the foreign schema so a stale `IMPORT FOREIGN SCHEMA`
/// from a prior aborted ingest doesn't shadow current source tables.
pub async fn setup(conn: &mut PgConnection) -> Result<(), MergeError> {
    tracing::info!("setting up postgres_fdw bridge to snapshot-source");

    conn.execute("CREATE EXTENSION IF NOT EXISTS postgres_fdw")
        .await?;

    let server_sql = format!(
        "CREATE SERVER IF NOT EXISTS merge_source_server \
         FOREIGN DATA WRAPPER postgres_fdw \
         OPTIONS (host '{FDW_HOST}', port '{FDW_PORT}', dbname '{FDW_DBNAME}')"
    );
    conn.execute(server_sql.as_str()).await?;

    let mapping_sql = format!(
        "CREATE USER MAPPING IF NOT EXISTS FOR CURRENT_USER \
         SERVER merge_source_server \
         OPTIONS (user '{FDW_USER}', password '{FDW_PASSWORD}')"
    );
    conn.execute(mapping_sql.as_str()).await?;

    conn.execute(format!("DROP SCHEMA IF EXISTS {FOREIGN_SCHEMA} CASCADE").as_str())
        .await?;
    conn.execute(format!("CREATE SCHEMA {FOREIGN_SCHEMA}").as_str())
        .await?;
    conn.execute(
        format!(
            "IMPORT FOREIGN SCHEMA public FROM SERVER merge_source_server INTO {FOREIGN_SCHEMA}"
        )
        .as_str(),
    )
    .await?;

    tracing::info!(schema = FOREIGN_SCHEMA, "FDW bridge ready");
    Ok(())
}

/// Drop the foreign schema, user mapping, and server. Call on the way
/// out of every `merge ingest`, success or failure post-setup.
pub async fn teardown(conn: &mut PgConnection) -> Result<(), MergeError> {
    conn.execute(format!("DROP SCHEMA IF EXISTS {FOREIGN_SCHEMA} CASCADE").as_str())
        .await?;
    conn.execute("DROP USER MAPPING IF EXISTS FOR CURRENT_USER SERVER merge_source_server")
        .await?;
    conn.execute("DROP SERVER IF EXISTS merge_source_server CASCADE")
        .await?;
    tracing::info!("FDW bridge torn down");
    Ok(())
}
