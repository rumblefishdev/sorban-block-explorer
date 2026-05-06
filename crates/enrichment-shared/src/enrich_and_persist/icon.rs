//! `assets.icon_url` enrichment — fetch the issuer's stellar.toml,
//! pull the matching `CURRENCIES[].image`, and write it to the asset row.
//!
//! Single source of truth for the live worker (one call per SQS message)
//! and any future local backfill (one call per streaming-SELECT row).
//!
//! ## Outcomes
//!
//! - **Success with image** — `UPDATE assets SET icon_url = '<url>'`.
//! - **Success without image** — issuer's TOML lists the asset but with
//!   no `image`, OR the TOML doesn't list the asset at all, OR the
//!   asset has no `home_domain` to fetch in the first place. Writes the
//!   empty-string sentinel `''` so re-runs short-circuit on
//!   `WHERE icon_url IS NULL` (a future `--force-retry` flag is
//!   captured in the task's Future Work).
//! - **Permanent fetch failure** — TOML 404, malformed TOML, body too
//!   large, non-UTF-8, malformed `home_domain`. Same `''` sentinel.
//! - **Transient fetch failure** — timeout / network / 5xx. Returns
//!   [`EnrichError::Transient`] so SQS retries.
//! - **DB failure** — returns [`EnrichError::Database`]. SQS retries.

use sqlx::{PgPool, Row};
use tracing::{debug, instrument, warn};

use super::EnrichError;
use crate::sep1::errors::Sep1Error;
use crate::sep1::{Sep1Fetcher, Sep1TomlParsed};

/// Matches `assets.icon_url VARCHAR(1024)`. Issuer-published URLs longer
/// than this are treated as a permanent SEP-1-side problem and the
/// sentinel is written instead — we don't want every UPDATE to fail
/// with a CHECK violation that bubbles up as an SQS retry.
const MAX_ICON_URL_BYTES: usize = 1024;

/// Run icon enrichment for one asset.
///
/// `asset_id` is `assets.id` (a `SERIAL`, hence i32). The fetcher should
/// be constructed once at process start (Lambda cold start, CLI bootstrap)
/// and reused across calls so the in-process LRU cache earns its keep.
#[instrument(skip(pool, fetcher), fields(asset_id))]
pub async fn enrich_asset_icon(
    pool: &PgPool,
    asset_id: i32,
    fetcher: &Sep1Fetcher,
) -> Result<(), EnrichError> {
    let row = sqlx::query(
        r#"
        SELECT
            a.asset_code,
            iss.account_id  AS issuer_strkey,
            iss.home_domain
        FROM assets a
        LEFT JOIN accounts iss ON iss.id = a.issuer_id
        WHERE a.id = $1
        "#,
    )
    .bind(asset_id)
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else {
        // Producer races (asset deleted between SQS publish and worker
        // consume) are not expected in MVP, but a missing row is treated
        // as a no-op so SQS doesn't keep redelivering.
        warn!("asset_id {asset_id} not found; acking SQS message");
        return Ok(());
    };

    let asset_code: Option<String> = row.try_get("asset_code")?;
    let issuer_strkey: Option<String> = row.try_get("issuer_strkey")?;
    let home_domain: Option<String> = row.try_get("home_domain")?;

    let Some(home_domain) = home_domain
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        // Native XLM, Soroban-only assets, or issuers that haven't
        // published a `home_domain` flag — nothing to fetch. Sentinel.
        write_sentinel(pool, asset_id).await?;
        return Ok(());
    };

    match fetcher.fetch(home_domain).await {
        Ok(parsed) => {
            let icon = find_icon(&parsed, asset_code.as_deref(), issuer_strkey.as_deref());
            match icon {
                Some(url) if !is_safe_icon_url(&url) => {
                    // Issuer published a URL with non-https:// scheme
                    // (e.g. `javascript:`, `data:`, `http://`). The
                    // frontend renders icon_url as `<img src="...">` so
                    // anything but https:// is either an XSS vector or
                    // a mixed-content failure. Treat as permanent —
                    // sentinel + warn.
                    warn!(
                        url_prefix = url.chars().take(20).collect::<String>(),
                        "icon URL not https:// — sentinel written (potential XSS)",
                    );
                    write_sentinel(pool, asset_id).await?;
                }
                Some(url) if url.len() > MAX_ICON_URL_BYTES => {
                    // Issuer published a URL too long for the column.
                    // Treat as permanent — sentinel + log.
                    warn!(
                        bytes = url.len(),
                        max = MAX_ICON_URL_BYTES,
                        "icon URL exceeds column limit; sentinel written"
                    );
                    write_sentinel(pool, asset_id).await?;
                }
                Some(url) => {
                    sqlx::query("UPDATE assets SET icon_url = $1 WHERE id = $2")
                        .bind(&url)
                        .bind(asset_id)
                        .execute(pool)
                        .await?;
                    debug!("icon_url set for asset_id {asset_id}");
                }
                None => {
                    // TOML parsed but no matching CURRENCIES[].image.
                    // Permanent for this `home_domain` snapshot — sentinel.
                    write_sentinel(pool, asset_id).await?;
                    debug!("no matching CURRENCIES[].image; sentinel written");
                }
            }
            Ok(())
        }
        Err(arc_err) => {
            if is_transient(&arc_err) {
                Err(EnrichError::Transient(arc_err.to_string()))
            } else {
                warn!("permanent SEP-1 fetch failure: {arc_err}; sentinel written");
                write_sentinel(pool, asset_id).await?;
                Ok(())
            }
        }
    }
}

async fn write_sentinel(pool: &PgPool, asset_id: i32) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE assets SET icon_url = '' WHERE id = $1")
        .bind(asset_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Reject anything that would be unsafe to render as an `<img src="...">`
/// on the frontend. Only `https://` URLs are accepted. `http://` is
/// rejected because the frontend is served over TLS (mixed-content
/// would block the request anyway). `javascript:`, `data:`, and any
/// other scheme are XSS vectors.
fn is_safe_icon_url(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    lower.starts_with("https://")
}

/// Find the first `CURRENCIES[]` row matching `(code, issuer)` and return
/// its `image` URL. Native XLM (no code, no issuer) intentionally never
/// matches — it has no SEP-1 entry.
fn find_icon(parsed: &Sep1TomlParsed, code: Option<&str>, issuer: Option<&str>) -> Option<String> {
    let (code, issuer) = (code?, issuer?);
    parsed
        .currencies
        .iter()
        .find(|c| c.code.as_deref() == Some(code) && c.issuer.as_deref() == Some(issuer))
        .and_then(|c| c.image.clone())
        .filter(|s: &String| !s.is_empty())
}

/// Decide whether a SEP-1 fetch error is worth retrying via SQS.
///
/// Network-layer (no HTTP status) and 5xx are transient — the issuer's
/// host may come back. 4xx and parse-level failures are permanent for
/// the current `home_domain` snapshot, written through as the empty
/// sentinel by the caller.
fn is_transient(err: &Sep1Error) -> bool {
    match err {
        Sep1Error::Timeout { .. } => true,
        Sep1Error::Http { source, .. } => match source.status() {
            Some(s) if s.is_server_error() => true,
            Some(_) => false, // 4xx, 3xx
            None => true,     // network-layer (DNS/TCP/TLS) — retry
        },
        Sep1Error::MissingHomeDomain
        | Sep1Error::MalformedHomeDomain { .. }
        | Sep1Error::BodyTooLarge { .. }
        | Sep1Error::NonUtf8Body
        | Sep1Error::MalformedToml { .. } => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sep1::dto::{Sep1Currency, Sep1TomlParsed};

    fn toml_with(code: &str, issuer: &str, image: Option<&str>) -> Sep1TomlParsed {
        Sep1TomlParsed {
            currencies: vec![Sep1Currency {
                code: Some(code.to_owned()),
                issuer: Some(issuer.to_owned()),
                desc: None,
                image: image.map(str::to_owned),
            }],
            documentation: None,
        }
    }

    #[test]
    fn find_icon_returns_image_when_match() {
        let parsed = toml_with("USDC", "GA1", Some("https://example.com/usdc.png"));
        let got = find_icon(&parsed, Some("USDC"), Some("GA1"));
        assert_eq!(got.as_deref(), Some("https://example.com/usdc.png"));
    }

    #[test]
    fn find_icon_returns_none_when_no_match() {
        let parsed = toml_with("USDC", "GA1", Some("https://example.com/usdc.png"));
        assert!(find_icon(&parsed, Some("EURC"), Some("GA1")).is_none());
        assert!(find_icon(&parsed, Some("USDC"), Some("GA2")).is_none());
    }

    #[test]
    fn find_icon_returns_none_when_native_xlm() {
        let parsed = toml_with("USDC", "GA1", Some("https://example.com/usdc.png"));
        assert!(find_icon(&parsed, None, None).is_none());
    }

    #[test]
    fn find_icon_returns_none_when_image_missing() {
        let parsed = toml_with("USDC", "GA1", None);
        assert!(find_icon(&parsed, Some("USDC"), Some("GA1")).is_none());
    }

    #[test]
    fn find_icon_treats_empty_image_as_missing() {
        let parsed = toml_with("USDC", "GA1", Some(""));
        assert!(find_icon(&parsed, Some("USDC"), Some("GA1")).is_none());
    }
}
