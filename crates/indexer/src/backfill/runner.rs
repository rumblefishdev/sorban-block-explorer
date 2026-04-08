use std::time::Duration;

use aws_sdk_ecs::Client as EcsClient;
use aws_sdk_ecs::types::{
    AssignPublicIp, AwsVpcConfiguration, ContainerOverride, KeyValuePair, LaunchType,
    NetworkConfiguration, TaskOverride,
};
use tokio::time::Instant;
use tracing::{info, warn};

use crate::range::LedgerRange;

#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    #[error("ECS RunTask failed for {range}: {message}")]
    RunTask { range: LedgerRange, message: String },
    #[error("ECS DescribeTasks failed: {0}")]
    DescribeTasks(String),
    #[error("ECS task {task_arn} failed for {range}: exit_code={exit_code}, reason={reason}")]
    TaskFailed {
        range: LedgerRange,
        task_arn: String,
        exit_code: i32,
        reason: String,
    },
    #[error("ECS task {task_arn} timed out after {timeout_secs}s waiting for STOPPED state")]
    Timeout { task_arn: String, timeout_secs: u64 },
}

/// Parameters for launching an ECS task.
pub struct RunTaskParams {
    pub cluster: String,
    pub task_definition: String,
    pub container_name: String,
    pub subnets: Vec<String>,
    pub security_groups: Vec<String>,
    pub task_timeout_secs: u64,
}

/// Outcome of a completed ECS task.
#[derive(Debug)]
pub struct TaskOutcome;

/// Launch a Galexie backfill ECS task for the given ledger range.
///
/// Returns the task ARN on success.
pub async fn launch_task(
    client: &EcsClient,
    params: &RunTaskParams,
    range: LedgerRange,
) -> Result<String, RunnerError> {
    let response = client
        .run_task()
        .cluster(&params.cluster)
        .task_definition(&params.task_definition)
        .launch_type(LaunchType::Fargate)
        .overrides(
            TaskOverride::builder()
                .container_overrides(
                    ContainerOverride::builder()
                        .name(&params.container_name)
                        .environment(
                            KeyValuePair::builder()
                                .name("START")
                                .value(range.start.to_string())
                                .build(),
                        )
                        .environment(
                            KeyValuePair::builder()
                                .name("END")
                                .value(range.end.to_string())
                                .build(),
                        )
                        .build(),
                )
                .build(),
        )
        .network_configuration(
            NetworkConfiguration::builder()
                .awsvpc_configuration(
                    AwsVpcConfiguration::builder()
                        .set_subnets(Some(params.subnets.clone()))
                        .set_security_groups(Some(params.security_groups.clone()))
                        .assign_public_ip(AssignPublicIp::Disabled)
                        .build()
                        .map_err(|e| RunnerError::RunTask {
                            range,
                            message: format!("invalid VPC config: {e}"),
                        })?,
                )
                .build(),
        )
        .count(1)
        .send()
        .await
        .map_err(|e| RunnerError::RunTask {
            range,
            message: e.to_string(),
        })?;

    if let Some(failure) = response.failures().first() {
        return Err(RunnerError::RunTask {
            range,
            message: format!(
                "ECS capacity failure — arn: {}, reason: {}",
                failure.arn().unwrap_or("?"),
                failure.reason().unwrap_or("unknown"),
            ),
        });
    }

    let task = response
        .tasks()
        .first()
        .ok_or_else(|| RunnerError::RunTask {
            range,
            message: "RunTask returned no tasks".to_string(),
        })?;

    let task_arn = task.task_arn().unwrap_or("unknown").to_string();

    info!(%range, task_arn, "ECS backfill task launched");
    Ok(task_arn)
}

/// Poll ECS until the task reaches STOPPED state, or until the timeout elapses.
pub async fn wait_for_task(
    client: &EcsClient,
    cluster: &str,
    task_arn: &str,
    range: LedgerRange,
    poll_interval: Duration,
    timeout: Duration,
) -> Result<TaskOutcome, RunnerError> {
    let deadline = Instant::now() + timeout;
    loop {
        tokio::time::sleep(poll_interval).await;

        if Instant::now() >= deadline {
            return Err(RunnerError::Timeout {
                task_arn: task_arn.to_string(),
                timeout_secs: timeout.as_secs(),
            });
        }

        let response = client
            .describe_tasks()
            .cluster(cluster)
            .tasks(task_arn)
            .send()
            .await
            .map_err(|e| RunnerError::DescribeTasks(e.to_string()))?;

        let task = response
            .tasks()
            .first()
            .ok_or_else(|| RunnerError::DescribeTasks(format!("task {task_arn} not found")))?;

        let status = task.last_status().unwrap_or("UNKNOWN");

        match status {
            "STOPPED" => {
                let container = task.containers().first();
                let exit_code = container.and_then(|c| c.exit_code()).unwrap_or(-1);
                let reason = task.stopped_reason().unwrap_or("unknown").to_string();

                if exit_code == 0 {
                    info!(%range, task_arn, "ECS task completed successfully");
                    return Ok(TaskOutcome);
                }

                warn!(%range, task_arn, exit_code, %reason, "ECS task failed");
                return Err(RunnerError::TaskFailed {
                    range,
                    task_arn: task_arn.to_string(),
                    exit_code,
                    reason,
                });
            }
            _ => {
                tracing::debug!(%range, task_arn, status, "ECS task still running");
            }
        }
    }
}
