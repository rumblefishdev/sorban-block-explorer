//! Reset the `postgres-snapshot-source` container and load a snapshot
//! into it via `pg_restore`. Runs as the first step of `merge ingest`
//! per task 0186 §Step 2.1.

use std::path::Path;
use std::time::{Duration, Instant};

use sqlx::Connection;
use tokio::process::Command;
use tokio::time::sleep;

use crate::error::MergeError;

/// docker-compose service name + volume name for the ephemeral
/// snapshot-source container. Tied 1:1 to docker-compose.yml.
pub const SERVICE: &str = "postgres-snapshot-source";
pub const VOLUME: &str = "sorban-block-explorer_pgdata-snapshot-source";

/// Drop the snapshot-source volume, recreate the container, wait for
/// healthy, then `pg_restore` the snapshot into it.
///
/// `snapshot_source_url` must point at the **host-side** port of
/// snapshot-source (e.g. `postgres://postgres:postgres@localhost:5437/...`).
pub async fn reset_and_restore(
    snapshot: &Path,
    snapshot_source_url: &str,
) -> Result<(), MergeError> {
    tracing::info!(service = SERVICE, "resetting snapshot-source container");

    // Stop + remove the container so the volume can be dropped. `--force`
    // on rm because the container may already be stopped from a prior run.
    run_subprocess("docker", &["compose", "stop", SERVICE]).await?;
    run_subprocess("docker", &["compose", "rm", "-f", SERVICE]).await?;

    // `volume rm` fails if the volume doesn't exist (first run / already
    // cleaned up). Tolerate that one specific case; surface anything else.
    let rm = Command::new("docker")
        .args(["volume", "rm", VOLUME])
        .output()
        .await?;
    if !rm.status.success() {
        let stderr = String::from_utf8_lossy(&rm.stderr);
        if !stderr.contains("No such volume") {
            return Err(MergeError::Subprocess {
                cmd: format!("docker volume rm {VOLUME}"),
                exit: rm.status.code(),
                stderr: stderr.into_owned(),
            });
        }
    }

    // Bring it back up (compose recreates the volume automatically).
    run_subprocess(
        "docker",
        &["compose", "--profile", "db-merge", "up", "-d", SERVICE],
    )
    .await?;

    wait_until_reachable(snapshot_source_url, Duration::from_secs(60)).await?;

    tracing::info!(snapshot = %snapshot.display(), "pg_restore into snapshot-source");
    run_subprocess(
        "pg_restore",
        &[
            "--dbname",
            snapshot_source_url,
            "--no-owner",
            "--no-privileges",
            "--single-transaction",
            snapshot.to_str().expect("snapshot path is valid utf-8"),
        ],
    )
    .await?;

    tracing::info!("snapshot-source ready");
    Ok(())
}

/// Tear down the snapshot-source container after a successful (or
/// failed) ingest. Volume is left in place so the user can inspect on
/// failure; the next `ingest` call drops it as its first step.
pub async fn stop() -> Result<(), MergeError> {
    run_subprocess("docker", &["compose", "stop", SERVICE]).await?;
    Ok(())
}

async fn run_subprocess(cmd: &str, args: &[&str]) -> Result<(), MergeError> {
    let output = Command::new(cmd).args(args).output().await?;
    if !output.status.success() {
        return Err(MergeError::Subprocess {
            cmd: format!("{cmd} {}", args.join(" ")),
            exit: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    Ok(())
}

/// Poll the URL until a connection succeeds or `timeout` elapses.
/// `pg_isready`-style probe via sqlx — avoids depending on the binary
/// being on PATH.
async fn wait_until_reachable(url: &str, timeout: Duration) -> Result<(), MergeError> {
    let deadline = Instant::now() + timeout;
    loop {
        match sqlx::PgConnection::connect(url).await {
            Ok(c) => {
                let _ = c.close().await;
                return Ok(());
            }
            Err(e) if Instant::now() >= deadline => return Err(MergeError::Db(e)),
            Err(_) => sleep(Duration::from_millis(500)).await,
        }
    }
}
