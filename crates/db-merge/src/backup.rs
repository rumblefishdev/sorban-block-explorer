//! Pre-merge `pg_dump` of the merge target — the only safe rollback for
//! cross-table corruption mid-merge. Per task 0186 §Step 2.4.
//!
//! Path is printed to stderr at the start of every `merge ingest`. The
//! user owns cleanup after a successful run.

use std::path::PathBuf;

use chrono::Utc;
use tokio::process::Command;

use crate::error::MergeError;

const BACKUP_DIR: &str = ".temp/db-merge-backups";

/// Take a `pg_dump --format=custom` of the target. Returns the full
/// path to the dump file. The directory is created if missing.
pub async fn dump_target(target_url: &str) -> Result<PathBuf, MergeError> {
    let dir = PathBuf::from(BACKUP_DIR);
    tokio::fs::create_dir_all(&dir).await?;

    let stamp = Utc::now().format("%Y%m%dT%H%M%SZ");
    let path = dir.join(format!("pre-merge-{stamp}.dump"));

    tracing::info!(path = %path.display(), "taking pre-merge pg_dump of target");

    let output = Command::new("pg_dump")
        .args([
            "--format=custom",
            "--file",
            path.to_str().expect("backup path is valid utf-8"),
            target_url,
        ])
        .output()
        .await?;

    if !output.status.success() {
        return Err(MergeError::Subprocess {
            cmd: "pg_dump".into(),
            exit: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }

    eprintln!(
        "db-merge: pre-merge target backup saved to {} — keep this until the merge is verified",
        path.display()
    );
    Ok(path)
}
