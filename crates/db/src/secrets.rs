//! AWS Secrets Manager credential resolution for RDS.
//!
//! Shared by all Lambda binaries (db-migrate, api, indexer) that connect
//! to PostgreSQL through RDS Proxy with Secrets Manager credentials.

use aws_sdk_secretsmanager::Client as SecretsClient;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};

/// Resolve a `DATABASE_URL` from Secrets Manager and an RDS endpoint.
///
/// The secret (identified by `secret_arn`) must contain `username` and
/// `password` fields in its JSON body (standard RDS secret format).
/// The database name is always `soroban_explorer`.
///
/// Returns a `postgres://` connection string with `sslmode=require`
/// (required by RDS Proxy).
pub async fn resolve_database_url(
    secret_arn: &str,
    rds_endpoint: &str,
) -> Result<String, ResolveError> {
    let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    let client = SecretsClient::new(&config);

    let response = client
        .get_secret_value()
        .secret_id(secret_arn)
        .send()
        .await
        .map_err(|e| ResolveError::SecretsManager(e.to_string()))?;

    let secret_string = response
        .secret_string()
        .ok_or(ResolveError::MissingField("secret_string"))?;

    let secret: serde_json::Value =
        serde_json::from_str(secret_string).map_err(ResolveError::Json)?;

    let username = secret["username"]
        .as_str()
        .ok_or(ResolveError::MissingField("username"))?;
    let password = secret["password"]
        .as_str()
        .ok_or(ResolveError::MissingField("password"))?;

    let encoded_username = utf8_percent_encode(username, NON_ALPHANUMERIC);
    let encoded_password = utf8_percent_encode(password, NON_ALPHANUMERIC);

    Ok(format!(
        "postgres://{encoded_username}:{encoded_password}@{rds_endpoint}:5432/soroban_explorer?sslmode=require"
    ))
}

#[derive(Debug)]
pub enum ResolveError {
    SecretsManager(String),
    Json(serde_json::Error),
    MissingField(&'static str),
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SecretsManager(e) => write!(f, "Secrets Manager error: {e}"),
            Self::Json(e) => write!(f, "failed to parse secret JSON: {e}"),
            Self::MissingField(field) => write!(f, "missing field in secret: {field}"),
        }
    }
}

impl std::error::Error for ResolveError {}
