use std::sync::Arc;
use std::time::Duration;

use aws_sdk_ecs::Client as EcsClient;
use aws_sdk_s3::Client as S3Client;
use tokio::sync::Semaphore;
use tracing::{error, info};

use crate::config::BackfillConfig;
use crate::range::{LedgerRange, find_gaps};
use crate::runner::{RunTaskParams, TaskOutcome, launch_task, wait_for_task};
use crate::scanner::scan_existing_ranges;

#[derive(Debug, thiserror::Error)]
pub enum OrchestrationError {
    #[error("S3 scan failed: {0}")]
    Scan(#[from] crate::scanner::ScanError),
}

/// Summary of a completed backfill run.
#[derive(Debug)]
pub struct BackfillSummary {
    pub total_batches: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub skipped: usize,
}

impl BackfillSummary {
    pub fn has_failures(&self) -> bool {
        self.failed > 0
    }
}

/// Run the backfill orchestration: scan S3, find gaps, launch ECS tasks.
pub async fn run(config: &BackfillConfig) -> Result<BackfillSummary, OrchestrationError> {
    let aws_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    let s3_client = S3Client::new(&aws_config);
    let ecs_client = EcsClient::new(&aws_config);

    let total_range = LedgerRange::new(config.start, config.end);
    let bucket = config.bucket_name();

    // Step 1: Scan S3 for existing files.
    info!(%total_range, bucket, "scanning S3 for existing ledger files");
    let existing = scan_existing_ranges(&s3_client, &bucket, "ledgers/").await?;

    // Step 2: Find gaps.
    let batches = find_gaps(total_range, &existing, config.batch_size);

    if batches.is_empty() {
        info!("all ledgers in range already present in S3");
        return Ok(BackfillSummary {
            total_batches: 0,
            succeeded: 0,
            failed: 0,
            skipped: 0,
        });
    }

    let total_ledgers: u64 = batches.iter().map(|b| b.len() as u64).sum();
    info!(
        batches = batches.len(),
        total_ledgers,
        concurrency = config.concurrency,
        "backfill plan ready"
    );

    for (i, batch) in batches.iter().enumerate() {
        info!(batch = i + 1, %batch, "planned batch");
    }

    // Step 3: Dry-run exits here.
    if config.dry_run {
        info!("dry-run mode — no tasks launched");
        return Ok(BackfillSummary {
            total_batches: batches.len(),
            succeeded: 0,
            failed: 0,
            skipped: batches.len(),
        });
    }

    // Step 4: Launch tasks with bounded concurrency.
    let params = Arc::new(RunTaskParams {
        cluster: config.cluster_name(),
        task_definition: config.task_def_family(),
        container_name: config.container_name.clone(),
        subnets: config.subnets.clone(),
        security_groups: config.security_groups.clone(),
        task_timeout_secs: config.task_timeout_secs,
    });

    let semaphore = Arc::new(Semaphore::new(config.concurrency));
    let poll_interval = Duration::from_secs(config.poll_interval_secs);

    let mut handles = Vec::with_capacity(batches.len());

    for (i, batch) in batches.into_iter().enumerate() {
        let permit = semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("semaphore closed unexpectedly");
        let ecs = ecs_client.clone();
        let params = Arc::clone(&params);

        handles.push(tokio::spawn(async move {
            let batch_num = i + 1;
            info!(batch = batch_num, %batch, "launching ECS task");

            let result = launch_and_wait(&ecs, &params, batch, poll_interval).await;

            match &result {
                Ok(_) => {
                    info!(batch = batch_num, %batch, "batch completed successfully");
                }
                Err(e) => {
                    error!(batch = batch_num, %batch, error = %e, "batch failed");
                }
            }

            drop(permit);
            (batch, result)
        }));
    }

    // Step 5: Collect results.
    let mut succeeded = 0usize;
    let mut failed = 0usize;

    for handle in handles {
        match handle.await {
            Ok((_batch, Ok(_))) => succeeded += 1,
            Ok((_batch, Err(e))) => {
                error!(error = %e, "task error");
                failed += 1;
            }
            Err(e) => {
                error!(error = %e, "tokio join error");
                failed += 1;
            }
        }
    }

    let summary = BackfillSummary {
        total_batches: succeeded + failed,
        succeeded,
        failed,
        skipped: 0,
    };

    info!(
        total = summary.total_batches,
        succeeded = summary.succeeded,
        failed = summary.failed,
        "backfill run complete"
    );

    Ok(summary)
}

async fn launch_and_wait(
    ecs: &EcsClient,
    params: &RunTaskParams,
    range: LedgerRange,
    poll_interval: Duration,
) -> Result<TaskOutcome, crate::runner::RunnerError> {
    let task_arn = launch_task(ecs, params, range).await?;
    let timeout = Duration::from_secs(params.task_timeout_secs);
    wait_for_task(
        ecs,
        &params.cluster,
        &task_arn,
        range,
        poll_interval,
        timeout,
    )
    .await
}
