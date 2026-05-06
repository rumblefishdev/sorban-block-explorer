//! Type-1 enrichment worker Lambda.
//!
//! SQS event source mapping → batches of `SqsEvent` records → per-record
//! dispatch by `kind` to the corresponding `enrich_*` function from the
//! shared `enrichment-shared` crate.
//!
//! Per task 0191:
//! - Worker writes are unconditional overwrites (no `WHERE col IS NULL`
//!   short-circuit). The duplicate-message contract lives in
//!   `enrichment_shared::enrich_and_persist::icon`.
//! - Batch failure model: each record is processed independently. A
//!   per-record failure is reported via `BatchItemFailures` so SQS
//!   redelivers only the failed messages, not the whole batch (the
//!   `ReportBatchItemFailures` response feature on the event source
//!   mapping).
//! - Cold start: build a single `Sep1Fetcher` (HTTP client + LRU cache)
//!   and a single `PgPool`; reuse both across handler invocations.
//!
//! Future kinds (`lp_tvl`, NFT metadata) plug in by adding a new arm to
//! the `match msg.kind` block and exposing the fn from `enrichment-shared`.

use std::sync::Arc;

use aws_lambda_events::event::sqs::{BatchItemFailure, SqsBatchResponse, SqsEvent, SqsMessage};
use enrichment_shared::enrich_and_persist::EnrichError;
use enrichment_shared::enrich_and_persist::icon::enrich_asset_icon;
use enrichment_shared::sep1::Sep1Fetcher;
use lambda_runtime::{Error, LambdaEvent, service_fn};
use serde::Deserialize;
use sqlx::PgPool;
use tracing::{error, info, instrument};

/// Per-message payload published by the indexer (Galaxy) Lambda.
///
/// Internally tagged on `kind` so each variant carries exactly the
/// fields it needs. Adding a future kind is one variant + one match
/// arm — the compiler enforces coverage. Unknown / malformed payloads
/// fail serde deserialisation and are treated as permanent (acked
/// without retry per [`handle_event`]).
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum EnrichmentMessage {
    Icon { asset_id: i32 },
}

struct WorkerState {
    pool: PgPool,
    sep1: Sep1Fetcher,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();

    info!("enrichment-worker cold start — resolving database credentials");

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

    let pool = db::pool::create_pool(&database_url)?;
    let sep1 = Sep1Fetcher::new()?;
    let state = Arc::new(WorkerState { pool, sep1 });

    info!("enrichment-worker ready — starting Lambda runtime");

    lambda_runtime::run(service_fn(move |event: LambdaEvent<SqsEvent>| {
        let state = Arc::clone(&state);
        async move { handle_event(event, state).await }
    }))
    .await
}

#[instrument(skip(event, state), fields(records = event.payload.records.len()))]
async fn handle_event(
    event: LambdaEvent<SqsEvent>,
    state: Arc<WorkerState>,
) -> Result<SqsBatchResponse, Error> {
    let mut failures = Vec::new();

    for record in event.payload.records {
        // SQS partial-batch reporting requires `item_identifier` to match
        // the record's messageId exactly — a wrong / synthetic value is
        // treated as "successfully processed" by the broker and the
        // record is silently deleted. A missing `message_id` is a Lambda
        // event-shape contract violation (AWS always sets it); fail the
        // whole invocation so the entire batch is redriven by SQS instead
        // of risking lost enrichment attempts.
        let Some(message_id) = record.message_id.clone() else {
            error!("SQS record missing message_id; failing invocation to force batch redrive");
            return Err("SQS record missing message_id".into());
        };

        match handle_record(&record, &state).await {
            Ok(()) => {}
            Err(RecordError::Permanent(e)) => {
                // Producer bug or corrupt SQS body. Retrying won't
                // fix it — ack the message so it doesn't burn the
                // DLQ on N retries. ERROR-level so log-based alarms
                // catch a sustained misshaped-publisher pattern.
                error!(
                    message_id = %message_id,
                    "permanent record error: {e}; acking without retry"
                );
            }
            Err(RecordError::Transient(e)) => {
                // Worth retrying (DB blip, transient SEP-1 5xx /
                // network). Report partial batch failure — SQS
                // redelivers per redrivePolicy.maxReceiveCount and
                // the DLQ alarm catches sustained outages.
                error!(
                    message_id = %message_id,
                    "transient enrichment failure: {e}; reporting partial batch failure"
                );
                failures.push(BatchItemFailure {
                    item_identifier: message_id,
                });
            }
        }
    }

    Ok(SqsBatchResponse {
        batch_item_failures: failures,
    })
}

async fn handle_record(record: &SqsMessage, state: &WorkerState) -> Result<(), RecordError> {
    let body = record
        .body
        .as_deref()
        .ok_or_else(|| RecordError::Permanent("SQS record had no body".to_owned()))?;
    let msg: EnrichmentMessage = serde_json::from_str(body)
        .map_err(|e| RecordError::Permanent(format!("malformed enrichment JSON: {e}")))?;

    match msg {
        EnrichmentMessage::Icon { asset_id } => {
            enrich_asset_icon(&state.pool, asset_id, &state.sep1).await?;
            Ok(())
        }
    }
}

/// Two-bucket error split mirrors the worker's retry semantics:
/// `Permanent` is acked (no retry), `Transient` triggers a SQS retry.
#[derive(Debug, thiserror::Error)]
enum RecordError {
    #[error("permanent: {0}")]
    Permanent(String),
    #[error("transient: {0}")]
    Transient(#[from] EnrichError),
}
