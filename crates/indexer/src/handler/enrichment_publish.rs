//! Type-1 enrichment SQS produce path (task 0191).
//!
//! After a ledger's persistence transaction commits, the indexer
//! publishes one SQS message per asset that needs runtime enrichment.
//! The downstream worker Lambda (`crates/enrichment-worker`) consumes
//! the queue, fetches the issuer's stellar.toml, and writes
//! `assets.icon_url`.
//!
//! ## Selection criteria
//!
//! After commit, query for asset rows that:
//!
//! - Match a `(code, issuer_strkey)` tuple or `contract_id` StrKey from
//!   the parser's `ExtractedAsset` slice for this ledger, AND
//! - Currently have `icon_url IS NULL` (un-enriched, including the
//!   sentinel `''` is **not** NULL — already-attempted permanent fails
//!   are skipped).
//!
//! This intentionally re-emits messages for *un-enriched but
//! pre-existing* asset rows that happened to be touched by this
//! ledger. The worker absorbs the cost of duplicates per the contract
//! in `enrichment_shared::enrich_and_persist::icon`. Once an asset is enriched it
//! drops out of this query naturally.
//!
//! ## Configuration
//!
//! `ENRICHMENT_QUEUE_URL` env var holds the SQS queue URL provisioned
//! by CDK. The indexer Lambda is a deploy-only artifact (CDK always
//! sets the variable), so the variable is **required** — a missing or
//! empty value fails Lambda cold start instead of silently disabling
//! the producer. CW `Init Errors` surfaces the misconfig immediately;
//! recovery is a fix-the-env-var redeploy. The trade-off accepted:
//! ingestion stops on enrichment misconfig (operator choice — explicit
//! signal preferred over partial availability).
//!
//! ## Failure model
//!
//! Publish failures are warn-logged and never propagated to the
//! handler. A dropped enrichment message is recoverable: a future
//! janitor (Future Work in 0191) re-emits stale rows, and the
//! operator-driven backfill (separate future task) drains
//! `WHERE icon_url IS NULL` directly. The persistence transaction has
//! already committed — fail-soft is correct here.

use aws_sdk_sqs::Client as SqsClient;
use aws_sdk_sqs::types::SendMessageBatchRequestEntry;
use sqlx::{PgPool, Row};
use tracing::{debug, error, info, instrument, warn};
use xdr_parser::types::ExtractedAsset;

const ENRICHMENT_QUEUE_URL_ENV: &str = "ENRICHMENT_QUEUE_URL";

/// SQS publisher for type-1 enrichment messages. Cheap to clone.
#[derive(Clone)]
pub struct Publisher {
    client: SqsClient,
    queue_url: String,
}

impl Publisher {
    /// Read `ENRICHMENT_QUEUE_URL` from the environment and build a
    /// publisher. Returns `Err` (string for the Lambda init error path)
    /// when the variable is missing or empty — the indexer Lambda is
    /// deploy-only and CDK always sets it, so a missing value is a
    /// misconfig that should fail cold start instead of silently
    /// disabling the producer.
    pub fn from_env(client: SqsClient) -> Result<Self, String> {
        let url = std::env::var(ENRICHMENT_QUEUE_URL_ENV)
            .map_err(|_| format!("{ENRICHMENT_QUEUE_URL_ENV} must be set"))?;
        if url.is_empty() {
            return Err(format!("{ENRICHMENT_QUEUE_URL_ENV} must not be empty"));
        }
        info!(queue_url = %url, "enrichment SQS publisher initialised");
        Ok(Self {
            client,
            queue_url: url,
        })
    }

    /// Look up un-enriched asset ids matching the parser's extracted
    /// assets and emit one `icon` SQS message per id.
    ///
    /// `extracted` is the parser's per-ledger `ExtractedAsset` slice.
    /// Empty slice short-circuits without touching the database.
    #[instrument(skip_all, fields(extracted = extracted.len()))]
    pub async fn publish_for_extracted_assets(&self, pool: &PgPool, extracted: &[ExtractedAsset]) {
        if extracted.is_empty() {
            return;
        }

        let asset_ids = match select_unenriched_asset_ids(pool, extracted).await {
            Ok(ids) => ids,
            Err(e) => {
                // ERROR (not WARN) so log-based alarms surface a sustained
                // outage. Indexer continues — persist_ledger has already
                // committed; the un-enriched rows are picked up by the
                // next ledger that touches them (still `WHERE icon_url
                // IS NULL`) or by a future janitor / backfill.
                error!(error = %e, "enrichment lookup failed; skipping SQS publish");
                return;
            }
        };

        if asset_ids.is_empty() {
            debug!("no un-enriched assets matched the extracted set; nothing to publish");
            return;
        }

        publish_icon_messages(&self.client, &self.queue_url, &asset_ids).await;
    }
}

/// Find ids of asset rows whose `icon_url IS NULL` and which match a
/// `(code, issuer_strkey)` tuple or `contract_id` StrKey from the
/// extracted set. Empty extracted set → empty result.
async fn select_unenriched_asset_ids(
    pool: &PgPool,
    extracted: &[ExtractedAsset],
) -> Result<Vec<i32>, sqlx::Error> {
    let mut codes: Vec<String> = Vec::new();
    let mut issuers: Vec<String> = Vec::new();
    let mut contracts: Vec<String> = Vec::new();
    for ext in extracted {
        if let (Some(code), Some(issuer)) = (&ext.asset_code, &ext.issuer_address) {
            codes.push(code.clone());
            issuers.push(issuer.clone());
        }
        if let Some(contract) = &ext.contract_id {
            contracts.push(contract.clone());
        }
    }
    if codes.is_empty() && contracts.is_empty() {
        return Ok(Vec::new());
    }

    let rows = sqlx::query(
        r#"
        SELECT DISTINCT a.id
        FROM assets a
        LEFT JOIN accounts iss          ON iss.id = a.issuer_id
        LEFT JOIN soroban_contracts sc  ON sc.id = a.contract_id
        WHERE a.icon_url IS NULL
          AND (
                -- classic_credit / sac match by (code, issuer_strkey) tuple
                (a.asset_code, iss.account_id) IN (
                    SELECT * FROM UNNEST($1::VARCHAR[], $2::VARCHAR[])
                )
                -- soroban / sac match by contract StrKey
                OR sc.contract_id = ANY($3::VARCHAR[])
              )
        "#,
    )
    .bind(&codes)
    .bind(&issuers)
    .bind(&contracts)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .filter_map(|r| r.try_get::<i32, _>("id").ok())
        .collect())
}

async fn publish_icon_messages(client: &SqsClient, queue_url: &str, asset_ids: &[i32]) {
    // SendMessageBatch caps at 10 messages per request.
    for chunk in asset_ids.chunks(10) {
        let mut entries = Vec::with_capacity(chunk.len());
        for (idx, id) in chunk.iter().enumerate() {
            // Build the JSON body via serde so future kinds with
            // string fields can't accidentally introduce injection.
            let body = serde_json::json!({ "kind": "icon", "asset_id": id }).to_string();
            debug!(kind = "icon", asset_id = id, "publishing enrichment msg");
            let entry = SendMessageBatchRequestEntry::builder()
                .id(format!("msg-{idx}-{id}"))
                .message_body(body)
                .build();
            match entry {
                Ok(entry) => entries.push(entry),
                Err(e) => warn!(error = %e, asset_id = id, "skipping malformed SQS entry"),
            }
        }
        if entries.is_empty() {
            continue;
        }
        let resp = client
            .send_message_batch()
            .queue_url(queue_url)
            .set_entries(Some(entries))
            .send()
            .await;
        match resp {
            Ok(out) => {
                let failed = out.failed.len();
                if failed > 0 {
                    // ERROR — partial-batch failure leaks msgs (those entries
                    // never reach the queue). Surface so a sustained pattern
                    // is alarm-able. Each failed entry's id + sender_fault +
                    // code is included for triage.
                    let failures: Vec<String> = out
                        .failed
                        .iter()
                        .map(|f| {
                            format!(
                                "{}:{}({})",
                                f.id,
                                f.code,
                                if f.sender_fault { "sender" } else { "receiver" }
                            )
                        })
                        .collect();
                    error!(
                        failed,
                        failures = ?failures,
                        "SQS send_message_batch reported partial failures",
                    );
                }
                debug!(
                    successful = out.successful.len(),
                    failed, "SQS batch published"
                );
            }
            // ERROR — full-batch failure (network, throttle, IAM, etc.).
            // Same recovery story as the lookup failure above: msgs are
            // lost for this ledger, recovered later via `WHERE icon_url
            // IS NULL` re-emission or backfill.
            Err(e) => error!(error = %e, "SQS send_message_batch failed"),
        }
    }
}
