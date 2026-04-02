//! Database migration Lambda for the Soroban block explorer.
//!
//! Invoked as a CloudFormation custom resource via CDK Provider.
//! On Create/Update: resolves credentials from Secrets Manager,
//! connects through RDS Proxy, and runs pending sqlx migrations.
//! On Delete: no-op (migrations are never auto-rolled-back).

use lambda_runtime::{Error, LambdaEvent, service_fn};
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;

const PHYSICAL_RESOURCE_ID: &str = "soroban-explorer-db-migrations";

async fn handler(event: LambdaEvent<Value>) -> Result<Value, Error> {
    let (payload, _context) = event.into_parts();

    let request_type = payload["RequestType"].as_str().unwrap_or("Create");

    tracing::info!(request_type, "migration handler invoked");

    match request_type {
        "Create" | "Update" => {
            let secret_arn = std::env::var("SECRET_ARN")
                .map_err(|_| "SECRET_ARN environment variable not set")?;
            let rds_endpoint = std::env::var("RDS_PROXY_ENDPOINT")
                .map_err(|_| "RDS_PROXY_ENDPOINT environment variable not set")?;

            let database_url =
                db::secrets::resolve_database_url(&secret_arn, &rds_endpoint).await?;

            let pool = PgPoolOptions::new()
                .max_connections(1)
                .connect(&database_url)
                .await?;

            tracing::info!("running migrations...");
            db::migrate::run_migrations(&pool).await?;
            tracing::info!("migrations completed successfully");

            pool.close().await;

            Ok(json!({
                "PhysicalResourceId": PHYSICAL_RESOURCE_ID,
                "Data": { "Message": "Migrations applied" }
            }))
        }
        "Delete" => {
            tracing::info!("delete event — no-op for migrations");
            Ok(json!({
                "PhysicalResourceId": PHYSICAL_RESOURCE_ID,
                "Data": { "Message": "No action on delete" }
            }))
        }
        other => {
            tracing::error!(request_type = other, "unknown request type — failing");
            Err(format!("unknown RequestType: {other}").into())
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();

    lambda_runtime::run(service_fn(handler)).await
}
