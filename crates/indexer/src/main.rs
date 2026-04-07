//! Ledger Processor Lambda for the Soroban block explorer.
//!
//! Processes LedgerCloseMeta payloads from S3 and persists structured data to PostgreSQL.

mod handler;

use aws_sdk_s3::Client as S3Client;
use lambda_runtime::{Error, service_fn};
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();

    info!("indexer cold start — resolving database credentials");

    // Resolve DATABASE_URL: prefer env var, fall back to Secrets Manager + RDS endpoint.
    let database_url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            let secret_arn = std::env::var("DB_SECRET_ARN")
                .map_err(|_| "either DATABASE_URL or DB_SECRET_ARN must be set")?;
            let rds_endpoint = std::env::var("RDS_ENDPOINT")
                .map_err(|_| "RDS_ENDPOINT must be set when using DB_SECRET_ARN")?;
            db::secrets::resolve_database_url(&secret_arn, &rds_endpoint)
                .await
                .map_err(|e| format!("failed to resolve database URL: {e}"))?
        }
    };

    let db_pool = db::pool::create_pool(&database_url)?;

    let aws_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    let s3_client = S3Client::new(&aws_config);

    let state = handler::HandlerState { s3_client, db_pool };

    info!("indexer ready — starting Lambda runtime");

    lambda_runtime::run(service_fn(|event| handler::handler(event, &state))).await
}
