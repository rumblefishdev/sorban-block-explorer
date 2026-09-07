//! Lambda handler for the Ledger Processor — SQS "doorbell" sequencer.
//!
//! The SQS message is a **content-free trigger** ("a file landed on S3, go
//! work"); its body is ignored. On each invocation the handler reconciles the
//! durable cursor — `max(sequence)` in ClickHouse — with what is contiguously
//! available on S3, and persists forward **oldest-first** starting at
//! `max + 1`, stopping at the first gap (next ledger not yet on S3) or when a
//! per-invocation time budget is reached. Each ledger is persisted via
//! [`db_clickhouse::persist::persist_ledger_clickhouse`] (open
//! `PartitionWriter` → write_ledger → commit), with the `ledgers` row written
//! last so a crash/timeout resumes cleanly from the new `max`.
//!
//! This gives **guaranteed ascending order without FIFO**: order comes from
//! the cursor + S3 contents (the source of truth), not from SQS delivery
//! order. It requires `reservedConcurrentExecutions = 1` — two concurrent
//! invocations would race the same cursor — which is load-bearing for
//! correctness, not a preference (see `compute-stack.ts`).
//!
//! Task 0244: the legacy 15-step PG flow (`handler::persist`) was removed.
//! The indexer drives ClickHouse only and contains no PG / sqlx code.

pub mod enrichment_publish;
pub mod process;

use aws_sdk_cloudwatch::Client as CloudWatchClient;
use aws_sdk_s3::Client as S3Client;
use db_clickhouse::persist::persist_ledger_clickhouse;
use domain::ClassificationCache;
use lambda_runtime::{Error, LambdaEvent};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tracing::{error, info, warn};

// ---------------------------------------------------------------------------
// SQS event types (S3 → SQS → Lambda, task 0241)
// ---------------------------------------------------------------------------
//
// The indexer is driven by an SQS event-source-mapping, not a direct S3
// trigger. Each message is a content-free **doorbell** — its `body` is
// ignored; it only wakes the reconcile (see module docs). We report
// per-message failures via the `ReportBatchItemFailures` response so SQS
// redelivers a failed doorbell (up to `maxReceiveCount`, then the DLQ);
// successful ones are acknowledged and deleted. `message_id` is kept only to
// name the batch-item-failure.

#[derive(Debug, Deserialize)]
pub struct SqsEvent {
    #[serde(rename = "Records")]
    pub records: Vec<SqsMessage>,
}

#[derive(Debug, Deserialize)]
pub struct SqsMessage {
    // Doorbell body is ignored (see module docs) — we only keep `messageId`
    // to name a batch-item-failure on a failed reconcile. Serde ignores the
    // unmapped `body` field in the SQS event JSON.
    #[serde(rename = "messageId")]
    pub message_id: String,
}

/// Partial-batch-failure response read by the SQS event-source-mapping.
/// An empty `batch_item_failures` acknowledges (deletes) every message.
#[derive(Debug, Serialize)]
pub struct SqsBatchResponse {
    #[serde(rename = "batchItemFailures")]
    pub batch_item_failures: Vec<BatchItemFailure>,
}

#[derive(Debug, Serialize)]
pub struct BatchItemFailure {
    #[serde(rename = "itemIdentifier")]
    pub item_identifier: String,
}

// ---------------------------------------------------------------------------
// Handler error
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum HandlerError {
    #[error("S3 download failed: {0}")]
    S3Download(String),
    #[error("parse error: {0}")]
    Parse(#[from] xdr_parser::ParseError),
    #[error("ClickHouse write failed: {0}")]
    ClickHouse(#[from] db_clickhouse::SchemaError),
}

// ---------------------------------------------------------------------------
// CH retry strategy
// ---------------------------------------------------------------------------

/// Mirrors the PG retry backoff shape — three retry attempts (four
/// wire calls total) with the same 50/200/800 ms cadence so operator
/// dashboards keep the same recovery envelope when something flaps on
/// the wire.
///
/// Applied per ledger inside the S3 batch — a transient hiccup on
/// ledger 3 retries just that ledger; ledgers 1–2 are already
/// committed. Hard error after retries → Lambda returns Err → S3
/// redelivery re-attempts the whole batch (`ReplacingMergeTree`
/// dedupes the 17 RMT tables on next merge).
const RETRY_BACKOFF_MS: [u64; 3] = [50, 200, 800];

// ---------------------------------------------------------------------------
// Shared state passed to every invocation
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct HandlerState {
    pub s3_client: S3Client,
    /// Ledger bucket name (`BUCKET_NAME` env, set by `compute-stack.ts`).
    /// The doorbell handler derives object keys from ledger numbers and
    /// HEAD/GETs them against this bucket — it does not read the S3 event.
    pub bucket: String,
    pub cw_client: CloudWatchClient,
    /// ClickHouse client (mTLS to Hetzner via Caddy). Construction lives
    /// in `main.rs` cold start; cloning is cheap (the underlying
    /// `hyper-util` pool is `Arc`-shared).
    pub ch_client: clickhouse::Client,
    /// Type-1 enrichment SQS publisher (task 0191; CH-repointed in 0231 step 6).
    /// **Live** — per batch it runs the ClickHouse anti-join against
    /// `*_enrichment` and publishes the misses (SEP-1 assets + NFT `token_uri`);
    /// see `enrichment_publish.rs`. (Operationally the consumer worker is gated
    /// off in prod until the 0301 rollout — the producer still publishes, so the
    /// worker must be enabled or the producer gated to avoid SQS age-out.)
    pub enrichment_publisher: enrichment_publish::Publisher,
    /// Task 0283 live G9 — per-warm-container NFT-routing verdict cache. Lazy,
    /// never caches `Other`; collapses the cross-ledger verdict lookup to one
    /// batched query per first sighting of a contract. Cloning shares the inner
    /// `Arc`, so all invocations on a warm container share the memo.
    pub classification_cache: ClassificationCache,
}

// ---------------------------------------------------------------------------
// Lambda entry point — SQS "doorbell" sequencer (task 0241)
// ---------------------------------------------------------------------------

/// Wall-time budget per invocation. A reconcile stops its contiguous run once
/// this elapses, leaving headroom under the Lambda timeout (600 s) for the
/// in-flight ledger (worst observed ~1.6 s) plus the final ack. Remaining
/// ledgers are picked up by the next doorbell — there are far more doorbells
/// (one per S3 file) than the handful of deadline-stops a backlog needs, so
/// the queue self-drains in order without re-enqueuing.
const RECONCILE_DEADLINE: Duration = Duration::from_secs(540);

/// Top-level Lambda handler. The SQS message is a content-free **doorbell**
/// ("something landed on S3, go work") — the body is ignored. Each invocation
/// runs [`reconcile`], which advances the CH cursor over the contiguous run
/// available on S3, oldest-first.
///
/// A reconcile that hits a gap or the time budget returns Ok (the doorbell is
/// acknowledged). A hard CH/S3 failure returns the message as a
/// batch-item-failure so SQS redelivers the doorbell and the next reconcile
/// resumes from the (possibly advanced) `max`.
pub async fn handler(
    event: LambdaEvent<SqsEvent>,
    state: &HandlerState,
) -> Result<SqsBatchResponse, Error> {
    let (payload, _ctx) = event.into_parts();
    let mut batch_item_failures = Vec::new();

    for msg in &payload.records {
        // Body irrelevant — every doorbell triggers the same reconcile.
        // batchSize is 1, so this loops once; if it ever isn't, a second
        // reconcile in the same batch is a cheap no-op (cursor already moved).
        if let Err(e) = reconcile(state).await {
            // Full error Display on purpose (policy reversed 2026-08-10,
            // lore-0455): the old sanitizer reduced the 0454 outage to the
            // undiagnosable label "ClickHouse error". Everything ClickHouse
            // could echo back is public on-chain data, so detail in logs
            // costs nothing and buys the post-incident "why".
            // `alarm` is a machine contract, not log copy: the
            // `indexer-ch-write-failures` CloudWatch metric filter keys on
            // `$.fields.alarm = "ch_write_failure"`. Reword the human
            // message freely; rename this field only together with
            // `infra/src/lib/stacks/cloudwatch-stack.ts` (the
            // declared-vs-emitted infra test enforces the pair).
            // `cause` exists because all three reconcile failure modes emit
            // this one alarm field, so an S3 outage or a parse bug pages as
            // "CH write failure" (lore-0455 review). Renaming the deployed
            // metric was judged not worth losing its history; the cause
            // field makes the first minute of an incident honest instead.
            let cause = match &e {
                HandlerError::S3Download(_) => "s3",
                HandlerError::Parse(_) => "parse",
                HandlerError::ClickHouse(_) => "clickhouse",
            };
            error!(
                alarm = "ch_write_failure",
                cause,
                message_id = %msg.message_id,
                error = %e,
                "reconcile failed — will redeliver doorbell"
            );
            batch_item_failures.push(BatchItemFailure {
                item_identifier: msg.message_id.clone(),
            });
        }
    }

    Ok(SqsBatchResponse {
        batch_item_failures,
    })
}

/// Persist the contiguous run of ledgers from `max(sequence) + 1` upward, in
/// strict ascending order, until either:
///   * the next ledger is **not yet on S3** (a gap) — return Ok and wait for a
///     future doorbell (this is the ordering barrier), or
///   * the **time budget** ([`RECONCILE_DEADLINE`]) is reached — return Ok;
///     the next doorbell resumes from the new (higher) `max`.
///
/// Returns Err only on a hard CH/S3 failure, which fails the doorbell so SQS
/// redelivers it. Already-persisted ledgers stay committed (the `ledgers` row
/// is written last per ledger), so a resume never reprocesses them.
async fn reconcile(state: &HandlerState) -> Result<(), HandlerError> {
    let start = Instant::now();

    let max_seq: i64 = state
        .ch_client
        .query("SELECT max(sequence) FROM ledgers")
        .fetch_one()
        .await
        .map_err(|e| HandlerError::ClickHouse(db_clickhouse::SchemaError::Query(e)))?;

    // Empty CH → `max()` is 0. Without a seeded baseline there is no floor to
    // start from (HEAD-probing up from ledger 1 would be millions of misses),
    // so no-op. Operationally CH is always seeded (snapshot / backfill) before
    // the live tail runs, so this is a guard, not a normal path.
    if max_seq <= 0 {
        warn!("ledgers table is empty (max=0) — no cursor to advance from; no-op");
        return Ok(());
    }

    let mut next = max_seq + 1;
    let mut persisted = 0u64;

    loop {
        // Check the budget BEFORE starting a ledger so we never begin one we
        // can't finish before the function timeout.
        if start.elapsed() >= RECONCILE_DEADLINE {
            info!(
                next,
                persisted, "reconcile hit time budget — stopping; next doorbell resumes"
            );
            return Ok(());
        }

        let key = ledger_s3_key(next);
        if !s3_object_exists(state, &key).await? {
            // `next` is not on S3 yet — stop. A future doorbell (when the file
            // lands) resumes here. This gate is what guarantees no gaps.
            if persisted == 0 {
                info!(next, "no new contiguous ledger on S3 — nothing to do");
            } else {
                info!(next, persisted, "reached gap on S3 — contiguous run done");
            }
            return Ok(());
        }

        info!(ledger = next, key = key.as_str(), "processing ledger");
        process_s3_object(state, &state.bucket, &key).await?;
        persisted += 1;
        next += 1;
    }
}

/// Derive the S3 object key for a ledger from Galexie's datastore naming
/// scheme (`ledgers_per_file = 1`, `files_per_partition = 64000`). Each name
/// component is prefixed with the one's-complement of its number in uppercase
/// hex, so S3's lexicographic listing is newest-first.
///
/// **Coupled to Galexie's scheme** (see `infra-hetzner` / the Galexie config
/// `[datastore_config.schema]`): if the partition size or prefix format
/// changes, update this in lockstep — a wrong key reads as a gap and stalls
/// the tail. Verified against a live key:
/// `L = 62528059` → `FC45E5FF--62528000-62591999/FC45E5C4--62528059.xdr.zst`.
fn ledger_s3_key(ledger: i64) -> String {
    const FILES_PER_PARTITION: i64 = 64_000;
    let part_start = (ledger / FILES_PER_PARTITION) * FILES_PER_PARTITION;
    let part_end = part_start + FILES_PER_PARTITION - 1;
    let part_prefix = 0xFFFF_FFFFu32 - part_start as u32;
    let file_prefix = 0xFFFF_FFFFu32 - ledger as u32;
    format!("{part_prefix:08X}--{part_start}-{part_end}/{file_prefix:08X}--{ledger}.xdr.zst")
}

/// HEAD the object: `true` if it exists, `false` on `NotFound`. Any other
/// error (network, permissions, throttling) propagates so the doorbell is
/// retried — mistaking a transient failure for a gap would stall the tail.
async fn s3_object_exists(state: &HandlerState, key: &str) -> Result<bool, HandlerError> {
    match state
        .s3_client
        .head_object()
        .bucket(&state.bucket)
        .key(key)
        .send()
        .await
    {
        Ok(_) => Ok(true),
        Err(err) => {
            if err.as_service_error().is_some_and(|e| e.is_not_found()) {
                Ok(false)
            } else {
                Err(HandlerError::S3Download(format!(
                    "head_object failed for {key}"
                )))
            }
        }
    }
}

/// Download, decompress, parse, and persist one S3 object containing a
/// batch of ledgers. Each ledger goes through its own
/// [`persist_ledger_clickhouse`] call (open writer → write → commit),
/// retried on transient HTTP errors with `[50, 200, 800] ms` backoff.
/// A hard failure on ledger N returns Err — already-committed ledgers
/// 1..N-1 stay in CH; S3 redelivery re-attempts the whole batch and
/// `ReplacingMergeTree` dedupes the 17 RMT tables.
async fn process_s3_object(
    state: &HandlerState,
    bucket: &str,
    key: &str,
) -> Result<(), HandlerError> {
    // Step 1: Download from S3
    let compressed = download_s3_object(&state.s3_client, bucket, key).await?;

    // Step 2: Decompress
    let xdr_bytes = xdr_parser::decompress_zstd(&compressed)?;

    // Step 3: Deserialize batch
    let batch = xdr_parser::deserialize_batch(&xdr_bytes)?;

    // Step 4: Per-ledger parse + persist with retry envelope. Aggregate
    // the enrichment-relevant slices so we pay the SQS lookup +
    // SendMessageBatch round-trips once per S3 record rather than once
    // per ledger (matches the pre-cutover handler shape; today both
    // publishes are stubbed, see `enrichment_publish.rs`).
    let mut batch_extracted_assets: Vec<xdr_parser::types::ExtractedAsset> = Vec::new();
    let mut batch_minted_nfts: Vec<xdr_parser::types::ExtractedNft> = Vec::new();

    for ledger_meta in batch.ledger_close_metas.iter() {
        let parsed = process::parse_ledger(ledger_meta);
        let ledger_sequence = parsed.ledger.sequence;
        let ledger_closed_at = parsed.ledger.closed_at;

        persist_with_retry(&state.ch_client, &parsed, &state.classification_cache).await?;
        publish_indexer_metrics(&state.cw_client, ledger_sequence, ledger_closed_at).await;

        // Mint = the token_uri-set event → the enrichment candidate. Non-mint
        // re-appearances (transfers) are already enriched or backfill's job.
        batch_minted_nfts.extend(
            parsed
                .nfts
                .into_iter()
                .filter(|n| n.minted_at_ledger.is_some()),
        );
        batch_extracted_assets.extend(parsed.assets);
    }

    // Enrichment publish — per-batch CH anti-join filter then SQS fan-out (task
    // 0231). See `enrichment_publish.rs`. Failures here never abort the batch:
    // the CH writes are already durable and the publish is fail-open.
    state
        .enrichment_publisher
        .publish_for_extracted_assets(&batch_extracted_assets)
        .await;
    state
        .enrichment_publisher
        .publish_for_minted_nfts(&batch_minted_nfts)
        .await;

    Ok(())
}

/// Wrap one `persist_ledger_clickhouse` call in the retry envelope.
/// Transient HTTP errors (network drop, timeout, Caddy / CH 5xx,
/// CH server-side transient exception codes) retry with backoff;
/// non-transient errors fail loud. Thin wrapper over
/// [`retry_with_backoff`] so the retry policy itself stays testable
/// without a live `clickhouse::Client`.
async fn persist_with_retry(
    client: &clickhouse::Client,
    parsed: &process::ParseOutput,
    classification_cache: &ClassificationCache,
) -> Result<(), HandlerError> {
    let ledger_sequence = parsed.ledger.sequence;
    let tx_count = parsed.transactions.len();

    let attempts = retry_with_backoff(
        ledger_sequence,
        &RETRY_BACKOFF_MS,
        is_transient_ch_error,
        || async {
            persist_ledger_clickhouse(
                client,
                &parsed.ledger,
                &parsed.transactions,
                &parsed.operations,
                &parsed.events,
                &parsed.invocations,
                &parsed.contract_interfaces,
                &parsed.contract_deployments,
                &parsed.account_states,
                &parsed.liquidity_pools,
                &parsed.pool_snapshots,
                &parsed.assets,
                &parsed.nfts,
                &parsed.nft_events,
                &parsed.lp_positions,
                &parsed.contract_metadata_writes,
                &parsed.soroban_token_balances,
                &parsed.pool_family_writes,
                &parsed.sac_overrides,
                classification_cache,
            )
            .await
            .map_err(HandlerError::ClickHouse)
        },
    )
    .await?;

    // Single success log per ledger. The recovery vs steady-state
    // distinction lives here (not inside `retry_with_backoff`) so we
    // emit exactly one line and keep `tx_count` in it — emitting both
    // a generic "persisted" and a "persisted after retry" would
    // double-count any throughput metric keyed on the message.
    if attempts > 0 {
        info!(
            ledger_sequence,
            tx_count,
            attempts = attempts + 1,
            "ledger persisted after retry"
        );
    } else {
        info!(ledger_sequence, tx_count, "ledger persisted");
    }
    Ok(())
}

// A `safe_error_message` sanitizer used to live here, reducing every error
// to a fixed label ("ClickHouse error") so no server-echoed row values could
// reach the logs. Removed 2026-08-10 (lore-0455): the database holds only
// public on-chain data, and the sanitizer's catch-all made the 2026-07-29
// outage undiagnosable — 19 minutes of failing writes logged as two words.
// Errors now log their full `Display`. Boundary of this decision: logs stay
// inside the account (CloudWatch, IAM-gated; nothing beyond alarm state
// reaches Slack). If logs ever ship to a third-party sink, re-open the
// redaction question before flipping that switch.

/// Generic retry envelope — drives `op` until it returns `Ok` or the
/// backoff schedule is exhausted. `is_transient` decides which errors
/// are eligible to retry; non-transient errors return immediately.
/// On success returns the number of retries performed (0 = succeeded
/// on the first attempt) so the caller can log a single recovery-aware
/// success line.
///
/// Pulled out of `persist_with_retry` so the loop control (attempt
/// counter, backoff sleep, transient classification) can be unit-
/// tested against a fake `op` that yields a scripted sequence of
/// `Result`s — the production caller uses the real
/// `persist_ledger_clickhouse` future.
///
/// **Note on `Send`**: no `Send` bound on `Fut` is intentional. The
/// Lambda binary drives this future from the single
/// `lambda_runtime::run` task; we never `tokio::spawn` it onto a
/// different worker. The `ParseOutput` reference captured by the
/// production closure is also `!Send` in some compositions, so
/// requiring `Send` here would force needless `Arc` wrappers
/// upstream. If a future caller needs to spawn, add the bound at
/// that callsite, not here.
async fn retry_with_backoff<F, Fut>(
    ledger_sequence: u32,
    backoff_ms: &[u64],
    is_transient: fn(&HandlerError) -> bool,
    mut op: F,
) -> Result<usize, HandlerError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<(), HandlerError>>,
{
    let mut attempt = 0usize;
    loop {
        match op().await {
            Ok(()) => return Ok(attempt),
            Err(err) => {
                if attempt < backoff_ms.len() && is_transient(&err) {
                    let delay = Duration::from_millis(backoff_ms[attempt]);
                    warn!(
                        ledger_sequence,
                        attempt,
                        backoff_ms = delay.as_millis() as u64,
                        error = %err,
                        "ledger persist hit transient CH error — retrying"
                    );
                    tokio::time::sleep(delay).await;
                    attempt += 1;
                    continue;
                }
                return Err(err);
            }
        }
    }
}

/// Wall-clock ingestion lag in seconds, clamped to 0. The clamp guards clock
/// skew where a validator's ledger close time is briefly ahead of this host's
/// clock (would otherwise report a spurious negative lag).
fn ingestion_lag_secs(now_secs: i64, ledger_closed_at_secs: i64) -> i64 {
    (now_secs - ledger_closed_at_secs).max(0)
}

/// Best-effort CW metric publish. Failures are warn-logged and do not abort the
/// batch — the metrics are operator visibility only; the authoritative state of
/// `LastProcessedLedgerSequence` is the `ledgers` table in CH.
///
/// Emits two datums in one call:
/// - `LastProcessedLedgerSequence` — last ledger written.
/// - `IngestionLagSeconds` — wall-clock seconds between ledger close and this
///   write (end-to-end ingestion lag; feeds the D3 AC3 dashboard + AC6 report).
async fn publish_indexer_metrics(
    cw_client: &CloudWatchClient,
    ledger_sequence: u32,
    ledger_closed_at_secs: i64,
) {
    use aws_sdk_cloudwatch::types::{Dimension, MetricDatum, StandardUnit};

    let env_name = std::env::var("ENV_NAME").unwrap_or_else(|_| "unknown".to_string());
    let dimension = Dimension::builder()
        .name("Environment")
        .value(&env_name)
        .build();

    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(ledger_closed_at_secs);
    let lag_secs = ingestion_lag_secs(now_secs, ledger_closed_at_secs);

    let sequence_datum = MetricDatum::builder()
        .metric_name("LastProcessedLedgerSequence")
        .dimensions(dimension.clone())
        .value(f64::from(ledger_sequence))
        .unit(StandardUnit::None)
        .build();
    let lag_datum = MetricDatum::builder()
        .metric_name("IngestionLagSeconds")
        .dimensions(dimension)
        .value(lag_secs as f64)
        .unit(StandardUnit::Seconds)
        .build();

    let result = cw_client
        .put_metric_data()
        .namespace("SorobanBlockExplorer/Indexer")
        .metric_data(sequence_datum)
        .metric_data(lag_datum)
        .send()
        .await;
    if let Err(e) = result {
        warn!(ledger_sequence, error = %e, "failed to publish indexer CW metrics");
    }
}

#[cfg(test)]
mod lag_tests {
    #[test]
    fn ingestion_lag_clamps_and_computes() {
        assert_eq!(super::ingestion_lag_secs(1_000, 970), 30);
        assert_eq!(super::ingestion_lag_secs(1_000, 1_005), 0); // clock skew → 0
    }
}

/// Classify a `HandlerError` as transient (eligible for the retry
/// envelope) vs hard (fail-loud). Mirrors the PG `retry_delay` shape
/// (40001 / 40P01 → transient) for the CH world.
///
/// * **Transport** — `Error::Network(_)` (hyper / IO failure on the
///   wire), `Error::TimedOut` (request-level timeout) → always retry.
/// * **`BadResponse`** — see [`is_retryable_bad_response`]: a denylist
///   of definitively permanent failures (HTTP 4xx, CH semantic
///   exception codes); everything else is retried.
/// * Other variants (serialization, config) → fail loud.
fn is_transient_ch_error(err: &HandlerError) -> bool {
    let HandlerError::ClickHouse(db_clickhouse::SchemaError::Query(ch_err)) = err else {
        return false;
    };
    match ch_err {
        clickhouse::error::Error::Network(_) | clickhouse::error::Error::TimedOut => true,
        clickhouse::error::Error::BadResponse(msg) => is_retryable_bad_response(msg),
        _ => false,
    }
}

/// Decide whether a `BadResponse` message is worth retrying.
///
/// **Denylist, not allowlist.** The `clickhouse` 0.15 crate does NOT
/// surface the HTTP status code separately: `BadResponse` carries the
/// server / proxy response **body verbatim** when it is non-empty, and
/// only falls back to `"NNN <Canonical-Reason>"` for empty bodies (see
/// `clickhouse-0.15.0/src/response.rs::collect_bad_response` +
/// `reason`). So a prefix match like `starts_with("502 ")` only catches
/// the *empty-body* case — a Caddy / CH 5xx that carries any body (or a
/// CH server error rendered as `"Code: NNN. DB::Exception…"`) would slip
/// through an allowlist and skip the retry envelope, tripping the DLQ on
/// routine infra turbulence.
///
/// For the live-tail indexer the INSERT payload is deterministic and
/// well-formed, so a `BadResponse` is almost always infra turbulence
/// (Caddy reload → 5xx, CH overload / restart, proxy hiccup). We
/// therefore **retry by default** and only fail loud on errors a retry
/// genuinely cannot fix: HTTP 4xx client errors and CH semantic
/// exception codes (schema / type / parse / auth). Worst case for a
/// permanent-but-unrecognised error is three wasted retries (~1 s) then
/// the same DLQ terminal state — availability over a marginal latency
/// cost.
fn is_retryable_bad_response(msg: &str) -> bool {
    !is_permanent_bad_response(msg)
}

/// HTTP 4xx prefixes the `clickhouse` crate renders for empty-body
/// client errors (`reason()` → `"NNN <Canonical-Reason>"`). Retrying a
/// 4xx never helps — bad request shape, auth, or routing.
const HTTP_4XX_PREFIXES: &[&str] = &[
    "400 ", "401 ", "403 ", "404 ", "405 ", "409 ", "413 ", "414 ", "415 ", "422 ", "431 ",
];

/// CH server-side exception codes that are deterministic for a given
/// INSERT — schema / query / auth problems a retry cannot fix. NOT
/// exhaustive: it's the set a well-formed indexer INSERT could
/// realistically hit (a mis-migrated schema, a cert mapped to the wrong
/// CH user, a parser/serialization bug). Unlisted codes fall through to
/// "retry" — acceptable, since a genuinely permanent unlisted code just
/// burns the retry budget before the DLQ.
const CH_PERMANENT_CODES: &[u32] = &[
    16,  // NO_SUCH_COLUMN_IN_TABLE
    27,  // CANNOT_PARSE_INPUT_ASSERTION_FAILED
    36,  // BAD_ARGUMENTS — malformed argument the same INSERT always reproduces
    47,  // UNKNOWN_IDENTIFIER
    53,  // TYPE_MISMATCH
    60,  // UNKNOWN_TABLE
    62,  // SYNTAX_ERROR
    73,  // UNKNOWN_FORMAT
    117, // INCORRECT_DATA — row fails CH-side validation (bad Decimal, enum, NULL into non-Null col); retry never fixes a deterministic row shape
    192, // UNKNOWN_USER
    193, // WRONG_PASSWORD
    194, // REQUIRED_PASSWORD
    497, // ACCESS_DENIED
    516, // AUTHENTICATION_FAILED
         // NOT listed: 49 LOGICAL_ERROR — ClickHouse raises it as a
         // catch-all for internal invariant violations, some of which are
         // transient (mid-mutation races, dictionary warmup after a
         // restart). Classifying it permanent would DLQ a ledger that a
         // retry could have landed; per the availability-over-latency
         // policy a code with any realistic transient case stays in the
         // default-retry path (worst case: a few wasted retries before the
         // DLQ on a genuinely permanent occurrence).
];

/// True when the message is a definitively permanent failure (HTTP 4xx
/// or a known CH semantic exception code).
fn is_permanent_bad_response(msg: &str) -> bool {
    if HTTP_4XX_PREFIXES.iter().any(|p| msg.starts_with(p)) {
        return true;
    }
    CH_PERMANENT_CODES
        .iter()
        .any(|code| ch_code_matches(msg, *code))
}

/// Match a CH exception body's leading `"Code: NNN"` token with a
/// trailing non-digit boundary, so `Code: 6` does not match `Code: 60`
/// and `Code: 159` does not match `Code: 1590`. CH bodies render as
/// `"Code: NNN. DB::Exception: …"`; the test fixtures also exercise the
/// `"Code: NNN, …"` comma form.
fn ch_code_matches(msg: &str, code: u32) -> bool {
    let needle = format!("Code: {code}");
    match msg.strip_prefix(needle.as_str()) {
        Some(rest) => !rest.starts_with(|c: char| c.is_ascii_digit()),
        None => false,
    }
}

/// Download an object from S3.
async fn download_s3_object(
    client: &S3Client,
    bucket: &str,
    key: &str,
) -> Result<Vec<u8>, HandlerError> {
    let resp = client
        .get_object()
        .bucket(bucket)
        .key(key)
        .send()
        .await
        .map_err(|e| HandlerError::S3Download(e.to_string()))?;

    let bytes = resp
        .body
        .collect()
        .await
        .map_err(|e| HandlerError::S3Download(e.to_string()))?
        .into_bytes()
        .to_vec();

    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    // -------------------------------------------------------------------
    // ledger_s3_key — Galexie datastore key derivation (correctness-critical:
    // a wrong key reads as a gap and stalls the tail)
    // -------------------------------------------------------------------

    #[test]
    fn ledger_s3_key_matches_galexie_scheme() {
        // Verified against a live key observed in the cutover S3 bucket.
        assert_eq!(
            ledger_s3_key(62_528_059),
            "FC45E5FF--62528000-62591999/FC45E5C4--62528059.xdr.zst"
        );
    }

    #[test]
    fn ledger_s3_key_partition_boundaries() {
        // First ledger of a partition: partition start == ledger.
        assert_eq!(
            ledger_s3_key(62_528_000),
            "FC45E5FF--62528000-62591999/FC45E5FF--62528000.xdr.zst"
        );
        // Last ledger of the same 64000-wide partition (62528000 + 63999).
        assert_eq!(
            ledger_s3_key(62_591_999),
            "FC45E5FF--62528000-62591999/FC44EC00--62591999.xdr.zst"
        );
        // First ledger of the next partition rolls the partition prefix.
        assert!(ledger_s3_key(62_592_000).starts_with("FC44EBFF--62592000-62655999/"));
    }

    // -------------------------------------------------------------------
    // retry_with_backoff — generic loop control
    // -------------------------------------------------------------------

    fn always_transient(_err: &HandlerError) -> bool {
        true
    }

    fn never_transient(_err: &HandlerError) -> bool {
        false
    }

    /// Cook up a fast `HandlerError` variant that doesn't construct CH
    /// internals — the classifier in `is_transient_ch_error` does its
    /// own job under separate tests; here we want pure loop-control
    /// scripting.
    fn dummy_err() -> HandlerError {
        HandlerError::S3Download("test-transient".into())
    }

    #[tokio::test]
    async fn retry_loop_success_on_first_try() {
        let calls = RefCell::new(0usize);
        let res = retry_with_backoff(
            42,
            &[1, 1, 1], // tiny backoffs to keep tests fast
            always_transient,
            || async {
                *calls.borrow_mut() += 1;
                Ok::<(), HandlerError>(())
            },
        )
        .await;
        // Returns 0 retries performed (succeeded on first attempt).
        assert_eq!(res.unwrap(), 0);
        assert_eq!(*calls.borrow(), 1);
    }

    #[tokio::test]
    async fn retry_loop_recovers_after_transient_errors() {
        let calls = RefCell::new(0usize);
        let res = retry_with_backoff(7, &[1, 1, 1], always_transient, || async {
            let n = {
                let mut c = calls.borrow_mut();
                *c += 1;
                *c
            };
            if n < 3 { Err(dummy_err()) } else { Ok(()) }
        })
        .await;
        // Succeeded on the 3rd call → 2 retries performed.
        assert_eq!(res.unwrap(), 2);
        assert_eq!(*calls.borrow(), 3);
    }

    #[tokio::test]
    async fn retry_loop_exhausts_after_max_attempts() {
        let calls = RefCell::new(0usize);
        let res = retry_with_backoff(99, &[1, 1, 1], always_transient, || async {
            *calls.borrow_mut() += 1;
            Err::<(), _>(dummy_err())
        })
        .await;
        assert!(res.is_err());
        // 1 initial + 3 retries = 4 wire calls total.
        assert_eq!(*calls.borrow(), 4);
    }

    #[tokio::test]
    async fn retry_loop_fails_loud_on_non_transient() {
        let calls = RefCell::new(0usize);
        let res = retry_with_backoff(11, &[1, 1, 1], never_transient, || async {
            *calls.borrow_mut() += 1;
            Err::<(), _>(dummy_err())
        })
        .await;
        assert!(res.is_err());
        // First call only — no retries because classifier rejects.
        assert_eq!(*calls.borrow(), 1);
    }

    #[tokio::test]
    async fn retry_loop_empty_backoff_means_zero_retries() {
        let calls = RefCell::new(0usize);
        let res = retry_with_backoff(0, &[], always_transient, || async {
            *calls.borrow_mut() += 1;
            Err::<(), _>(dummy_err())
        })
        .await;
        assert!(res.is_err());
        assert_eq!(*calls.borrow(), 1);
    }

    // -------------------------------------------------------------------
    // is_retryable_bad_response — classifier substrings
    // -------------------------------------------------------------------

    /// 5xx surfaces — both the empty-body `"NNN <reason>"` shape and
    /// (crucially) the non-empty-body shapes that an allowlist prefix
    /// match would have missed.
    #[test]
    fn is_retryable_bad_response_recognises_5xx() {
        // Empty-body form rendered by the crate's `reason()`.
        assert!(is_retryable_bad_response("502 Bad Gateway"));
        assert!(is_retryable_bad_response("503 Service Unavailable"));
        assert!(is_retryable_bad_response("504 Gateway Timeout"));
        assert!(is_retryable_bad_response("507 Insufficient Storage"));
        // Non-empty-body forms — the regression the denylist fixes.
        // Caddy / proxy 5xx carrying an HTML or text body, and a CH
        // server-side 5xx rendered as a verbatim body. An allowlist
        // prefix("502 ") would have returned false here.
        assert!(is_retryable_bad_response(
            "<html><head><title>502 Bad Gateway</title></head>...</html>"
        ));
        assert!(is_retryable_bad_response(
            "upstream connect error or disconnect/reset before headers"
        ));
    }

    #[test]
    fn is_retryable_bad_response_recognises_ch_transient_codes() {
        assert!(is_retryable_bad_response(
            "Code: 159, DB::Exception: TIMEOUT_EXCEEDED ..."
        ));
        assert!(is_retryable_bad_response(
            "Code: 252, DB::Exception: TOO_MANY_PARTS ..."
        ));
        assert!(is_retryable_bad_response("Code: 999"));
        // CH bodies use a trailing period after the code.
        assert!(is_retryable_bad_response(
            "Code: 202. DB::Exception: Too many simultaneous queries"
        ));
    }

    #[test]
    fn is_retryable_bad_response_skips_semantic_errors() {
        // 4xx — client error, retry won't help.
        assert!(!is_retryable_bad_response("400 Bad Request"));
        assert!(!is_retryable_bad_response("403 Forbidden"));
        assert!(!is_retryable_bad_response("404 Not Found"));
        // Type / parse / table / auth errors from CH — semantic, no retry.
        assert!(!is_retryable_bad_response(
            "Code: 60, DB::Exception: UNKNOWN_TABLE ..."
        ));
        assert!(!is_retryable_bad_response(
            "Code: 27, DB::Exception: CANNOT_PARSE_INPUT_ASSERTION_FAILED ..."
        ));
        assert!(!is_retryable_bad_response(
            "Code: 53, DB::Exception: TYPE_MISMATCH ..."
        ));
        assert!(!is_retryable_bad_response(
            "Code: 516. DB::Exception: ... AUTHENTICATION_FAILED"
        ));
    }

    /// Data-shape errors are deterministic for a given INSERT — a retry
    /// re-sends the same bad row and fails identically. They must be
    /// classed permanent so the budget isn't burned before the DLQ.
    #[test]
    fn is_retryable_bad_response_skips_data_shape_errors() {
        assert!(!is_retryable_bad_response(
            "Code: 117. DB::Exception: INCORRECT_DATA: Cannot parse ..."
        ));
        assert!(!is_retryable_bad_response(
            "Code: 36, DB::Exception: BAD_ARGUMENTS ..."
        ));
    }

    /// `49 LOGICAL_ERROR` is a CH catch-all with transient cases
    /// (mid-mutation race, dictionary warmup) — it must NOT be classed
    /// permanent, so a transient occurrence still gets the retry
    /// envelope rather than going straight to the DLQ.
    #[test]
    fn is_retryable_bad_response_retries_logical_error() {
        assert!(is_retryable_bad_response(
            "Code: 49. DB::Exception: LOGICAL_ERROR ..."
        ));
    }

    /// The full error Display must survive into the rendered string —
    /// the sanitizer that used to strip it made the 2026-07-29 outage
    /// undiagnosable. Guards against a well-meaning re-introduction.
    #[test]
    fn handler_error_display_keeps_ch_detail() {
        let body = "Code: 252. DB::Exception: Too many parts (5000)";
        let err = HandlerError::ClickHouse(db_clickhouse::SchemaError::Query(
            clickhouse::error::Error::BadResponse(body.to_string()),
        ));
        let rendered = err.to_string();
        assert!(rendered.contains("Code: 252"));
        assert!(rendered.contains("Too many parts"));
    }

    /// Prefix-boundary: `Code: 6` must not match `Code: 60`, and a
    /// transient `Code: 159` must not be shadowed by a permanent
    /// `Code: 1` / `Code: 16` entry (and vice-versa).
    #[test]
    fn ch_code_boundary_is_exact() {
        // `60` is permanent; `600` (hypothetical, unlisted) is not —
        // boundary check must not let `60` swallow `600`.
        assert!(is_retryable_bad_response(
            "Code: 600. DB::Exception: hypothetical transient"
        ));
        // `16` permanent must not match `169` (unlisted → retry).
        assert!(is_retryable_bad_response("Code: 169. DB::Exception: x"));
        // Exact permanent still caught.
        assert!(!is_retryable_bad_response("Code: 16. DB::Exception: x"));
    }

    /// Unrecognised / opaque BadResponse bodies default to retryable
    /// — availability over a marginal retry cost (the live-tail INSERT
    /// is deterministic, so an opaque error is almost always infra).
    #[test]
    fn is_retryable_bad_response_defaults_unknown_to_retry() {
        assert!(is_retryable_bad_response("some opaque caddy body"));
        assert!(is_retryable_bad_response(""));
        assert!(is_retryable_bad_response("Code: 241 MEMORY_LIMIT_EXCEEDED"));
    }
}
