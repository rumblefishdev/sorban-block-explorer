use clap::Parser;

/// First ledger of Protocol 20 (Soroban mainnet, Feb 20 2024).
/// The task spec references ~50,692,993 as a rough estimate; the precise
/// activation checkpoint is 50,457,424. Using the earlier value ensures
/// no Soroban ledgers are missed.
pub const SOROBAN_ACTIVATION_LEDGER: u32 = 50_457_424;

#[derive(Parser, Debug)]
#[command(
    name = "backfill",
    about = "Backfill orchestrator for Stellar ledger indexing via Galexie ECS tasks"
)]
pub struct BackfillConfig {
    /// First ledger to backfill (inclusive).
    #[arg(long, default_value_t = SOROBAN_ACTIVATION_LEDGER)]
    pub start: u32,

    /// Last ledger to backfill (inclusive).
    #[arg(long)]
    pub end: u32,

    /// Number of ledgers per Galexie ECS task.
    #[arg(long, default_value_t = 10_000)]
    pub batch_size: u32,

    /// Maximum concurrent ECS tasks.
    #[arg(long, default_value_t = 3)]
    pub concurrency: usize,

    /// Environment name (e.g. "staging", "production").
    /// Derives cluster, task definition, and bucket names.
    #[arg(long, env = "ENV_NAME")]
    pub env_name: String,

    /// ECS cluster name override. Defaults to "{env_name}-ingestion".
    #[arg(long)]
    pub cluster: Option<String>,

    /// Backfill task definition family override. Defaults to "{env_name}-galexie-backfill".
    #[arg(long)]
    pub task_def: Option<String>,

    /// S3 bucket name override. Defaults to "{env_name}-stellar-ledger-data".
    #[arg(long)]
    pub bucket: Option<String>,

    /// Galexie container name in the task definition.
    #[arg(long, default_value = "Galexie")]
    pub container_name: String,

    /// Subnets for ECS tasks (comma-separated).
    #[arg(long, value_delimiter = ',')]
    pub subnets: Vec<String>,

    /// Security group IDs for ECS tasks (comma-separated).
    #[arg(long, value_delimiter = ',')]
    pub security_groups: Vec<String>,

    /// Print the plan without launching tasks.
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,

    /// Seconds between ECS task status polls.
    #[arg(long, default_value_t = 30)]
    pub poll_interval_secs: u64,

    /// Maximum seconds to wait for a single ECS task to reach STOPPED state.
    #[arg(long, default_value_t = 3600)]
    pub task_timeout_secs: u64,
}

impl BackfillConfig {
    pub fn cluster_name(&self) -> String {
        self.cluster
            .clone()
            .unwrap_or_else(|| format!("{}-ingestion", self.env_name))
    }

    pub fn task_def_family(&self) -> String {
        self.task_def
            .clone()
            .unwrap_or_else(|| format!("{}-galexie-backfill", self.env_name))
    }

    pub fn bucket_name(&self) -> String {
        self.bucket
            .clone()
            .unwrap_or_else(|| format!("{}-stellar-ledger-data", self.env_name))
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.start > self.end {
            return Err(format!(
                "--start ({}) must be <= --end ({})",
                self.start, self.end
            ));
        }
        if self.batch_size == 0 {
            return Err("--batch-size must be > 0".to_string());
        }
        if self.concurrency == 0 {
            return Err("--concurrency must be > 0".to_string());
        }
        if self.subnets.is_empty() {
            return Err("--subnets is required".to_string());
        }
        if self.security_groups.is_empty() {
            return Err("--security-groups is required".to_string());
        }
        Ok(())
    }
}
