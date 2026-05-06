//! Ledger Processor Lambda for the Soroban block explorer.
//!
//! Processes LedgerCloseMeta payloads from S3 and persists structured data to PostgreSQL.

mod handler;

use aws_sdk_cloudwatch::Client as CloudWatchClient;
use aws_sdk_s3::Client as S3Client;
use aws_sdk_sqs::Client as SqsClient;
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
            let secret_arn = std::env::var("SECRET_ARN")
                .map_err(|_| "either DATABASE_URL or SECRET_ARN must be set")?;
            let rds_endpoint = std::env::var("RDS_PROXY_ENDPOINT")
                .map_err(|_| "RDS_PROXY_ENDPOINT must be set when using SECRET_ARN")?;
            db::secrets::resolve_database_url(&secret_arn, &rds_endpoint)
                .await
                .map_err(|e| format!("failed to resolve database URL: {e}"))?
        }
    };

    let db_pool = db::pool::create_pool(&database_url)?;

    let aws_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    let s3_client = S3Client::new(&aws_config);
    let cw_client = CloudWatchClient::new(&aws_config);
    let sqs_client = SqsClient::new(&aws_config);

    // Type-1 enrichment publisher (task 0191). Required env var —
    // missing/empty value fails Lambda init so the misconfig surfaces
    // immediately via CW Init Errors instead of silently dropping
    // enrichment messages on every ledger.
    let enrichment_publisher = handler::enrichment_publish::Publisher::from_env(sqs_client)?;

    let state = handler::HandlerState {
        s3_client,
        cw_client,
        db_pool,
        classification_cache: handler::persist::ClassificationCache::new(),
        enrichment_publisher,
    };

    info!("indexer ready — starting Lambda runtime");

    lambda_runtime::run(service_fn(|event| handler::handler(event, &state))).await
}
