//! `Sep1Fetcher` — fail-soft LRU-cached HTTP client for issuer stellar.toml files.
//!
//! Hot path: `fetch(home_domain)` returns `Arc<Sep1TomlParsed>` from the in-process
//! cache when warm; on a miss it issues a single GET to
//! `https://{home_domain}/.well-known/stellar.toml`, caps the body at the SEP-1
//! 100 KB limit, parses the TOML and stores the result. Every error path returns
//! a `Sep1Error` that the consumer maps silently to null fields — the API never
//! 5xx's because of an enrichment failure.
//!
//! Cache: `moka::future::Cache` (built via `crate::cache::ttl_future_cache`)
//! with a 24 h TTL and 1024-entry capacity; warm only within a single Lambda
//! container, lost on cold start. The future variant lets us collapse
//! concurrent cold-cache misses for the same `home_domain` onto a single
//! in-flight HTTP fetch via `try_get_with`.
//!
//! Built-in SSRF guards (best-effort, not airtight):
//!   - `home_domain` must be RFC 1035-style (ASCII alphanumeric / `.` / `-`).
//!   - `home_domain` must not parse as a literal IP address (rejects
//!     `127.0.0.1`, `192.168.0.1`, `[::1]`, `169.254.169.254`).
//!   - HTTP redirects are **not followed** (`Policy::limited(0)`). SEP-1
//!     doesn't require redirect support, and following any 30x would
//!     re-open the SSRF gate the initial `validate_host` closes — a
//!     redirect to `127.0.0.1` / `169.254.169.254` / a private IP
//!     literal would bypass the up-front host check. With `limited(0)`
//!     reqwest maps any 3xx into a `reqwest::Error` (which we surface
//!     as `Sep1Error::Http` and the consumer treats fail-soft as null
//!     fields), so legitimate issuers behind apex↔www-style redirects
//!     simply get null `description` / `home_page` until their
//!     `home_domain` flag matches the canonical TOML host directly.
//!   - DNS-resolved private addresses are NOT blocked at this layer; deeper
//!     SSRF protection (resolve + check against RFC 1918 / 6598 / link-local
//!     ranges) is a follow-up if the threat model demands it.

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use reqwest::redirect::Policy;
use tracing::instrument;

use crate::cache::{FutureCache, ttl_future_cache};

use super::dto::Sep1TomlParsed;
use super::errors::Sep1Error;

/// SEP-1 caps stellar.toml at 100 KB; reject without buffering the rest.
const MAX_BODY_BYTES: usize = 100 * 1024;

/// Per-host TCP connect timeout. Tight so a hung issuer can't burn the
/// whole request budget on connect alone.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);

/// Total per-request budget (connect + TLS + headers + body). Combined
/// with the per-Lambda enrichment fan-out budget this stays well under
/// the API Gateway 29 s ceiling.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);

/// Cache TTL. Issuer stellar.toml files change infrequently; 24 h trades
/// freshness for hit rate. Warm cache survives only inside a single Lambda
/// container.
const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Max distinct issuer domains held warm per container.
const CACHE_CAPACITY: u64 = 1024;

const USER_AGENT: &str = concat!("soroban-block-explorer/", env!("CARGO_PKG_VERSION"));

/// HTTP fetcher for SEP-1 stellar.toml files.
///
/// Cheap to clone: both the inner `reqwest::Client` and the
/// `moka::future::Cache` are `Arc`-backed. Construct once at Lambda
/// cold-start, reuse from `AppState`.
#[derive(Clone)]
pub struct Sep1Fetcher {
    client: reqwest::Client,
    cache: FutureCache<String, Arc<Sep1TomlParsed>>,
}

impl Sep1Fetcher {
    /// Construct a fetcher with the production HTTP / cache configuration.
    pub fn new() -> Result<Self, reqwest::Error> {
        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            // No redirect-following: SEP-1 doesn't require them, and any
            // 30x would let the issuer bypass `validate_host` by bouncing
            // us to a loopback / link-local / RFC 1918 host after the
            // initial check passed. `limited(0)` (vs `none()`) makes
            // reqwest surface 3xx as `reqwest::Error::TooManyRedirects`
            // so the existing `Sep1Error::Http` mapping handles it
            // without a dedicated variant.
            .redirect(Policy::limited(0))
            .user_agent(USER_AGENT)
            .build()?;
        let cache = ttl_future_cache(CACHE_TTL, CACHE_CAPACITY);
        Ok(Self { client, cache })
    }

    /// Fetch and parse the issuer's stellar.toml.
    ///
    /// Cache hits return the warm `Arc`. Cold misses validate the host,
    /// issue a single GET, cap the body, deserialise the TOML, then cache
    /// the parsed result keyed by the lowercase domain. Concurrent misses
    /// for the same domain collapse onto a single in-flight load via
    /// `try_get_with`. Errors are wrapped in `Arc<Sep1Error>` (moka's
    /// signature for shared failure values) and **not** cached, so a
    /// transient failure on a cold key does not poison subsequent requests.
    #[instrument(skip(self), fields(home_domain = %home_domain))]
    pub async fn fetch(&self, home_domain: &str) -> Result<Arc<Sep1TomlParsed>, Arc<Sep1Error>> {
        let key = home_domain.trim().to_ascii_lowercase();
        let client = self.client.clone();
        self.cache
            .try_get_with(key.clone(), async move {
                validate_host(&key)?;
                Ok::<_, Sep1Error>(Arc::new(fetch_uncached(&client, &key).await?))
            })
            .await
    }
}

async fn fetch_uncached(client: &reqwest::Client, host: &str) -> Result<Sep1TomlParsed, Sep1Error> {
    let url = format!("https://{host}/.well-known/stellar.toml");

    let resp = client.get(&url).send().await.map_err(|e| {
        if e.is_timeout() {
            Sep1Error::Timeout {
                host: host.to_owned(),
            }
        } else {
            Sep1Error::Http {
                host: host.to_owned(),
                source: e,
            }
        }
    })?;

    if !resp.status().is_success() {
        // `error_for_status` consumes the response and turns the status
        // into a `reqwest::Error`. Safe to unwrap — we just checked
        // `is_success() == false`.
        let err = resp.error_for_status().expect_err("status was not success");
        return Err(Sep1Error::Http {
            host: host.to_owned(),
            source: err,
        });
    }

    let bytes = capped_body(resp, host).await?;
    let text = std::str::from_utf8(&bytes).map_err(|_| Sep1Error::NonUtf8Body)?;
    toml::from_str::<Sep1TomlParsed>(text).map_err(|source| Sep1Error::MalformedToml { source })
}

/// RFC 1035-style hostname check + IP-literal rejection.
///
/// Accepts: ASCII alphanumeric, `.`, `-`. Rejects empty, anything with a
/// scheme / path / port / colon, and any string that parses as `IpAddr`.
fn validate_host(host: &str) -> Result<(), Sep1Error> {
    if host.is_empty() {
        return Err(Sep1Error::MalformedHomeDomain {
            host: host.to_owned(),
        });
    }
    if !host
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
    {
        return Err(Sep1Error::MalformedHomeDomain {
            host: host.to_owned(),
        });
    }
    if host.parse::<IpAddr>().is_ok() {
        return Err(Sep1Error::MalformedHomeDomain {
            host: host.to_owned(),
        });
    }
    Ok(())
}

/// Stream the body chunk-by-chunk; bail out if the running total crosses
/// `MAX_BODY_BYTES` before fully buffering.
async fn capped_body(mut resp: reqwest::Response, host: &str) -> Result<Vec<u8>, Sep1Error> {
    let mut buf: Vec<u8> = Vec::with_capacity(8 * 1024);
    loop {
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                if buf.len().saturating_add(chunk.len()) > MAX_BODY_BYTES {
                    return Err(Sep1Error::BodyTooLarge {
                        limit: MAX_BODY_BYTES,
                    });
                }
                buf.extend_from_slice(&chunk);
            }
            Ok(None) => return Ok(buf),
            // Mirror the send-side error mapping: a stalled body read past
            // the per-request budget should surface as `Timeout`, not the
            // generic `Http` bucket — otherwise oncall can't tell connect
            // / send-side hangs from body-side hangs.
            Err(e) if e.is_timeout() => {
                return Err(Sep1Error::Timeout {
                    host: host.to_owned(),
                });
            }
            Err(e) => {
                return Err(Sep1Error::Http {
                    host: host.to_owned(),
                    source: e,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! Unit tests for `validate_host`. The full HTTP path (`fetch_uncached`,
    //! `capped_body`, error mapping, cache wrap) is intentionally not covered
    //! by automated tests in-tree — see task 0188 §"Out of Scope" for the
    //! rationale. A real-issuer smoke test against e.g. `ultrastellar.com`
    //! is deferred to a follow-up.

    use super::*;

    #[test]
    fn validate_host_accepts_normal_dns_names() {
        assert!(validate_host("ultrastellar.com").is_ok());
        assert!(validate_host("api.example.co.uk").is_ok());
        assert!(validate_host("issuer-2.example.com").is_ok());
    }

    #[test]
    fn validate_host_rejects_empty() {
        assert!(matches!(
            validate_host(""),
            Err(Sep1Error::MalformedHomeDomain { .. })
        ));
    }

    #[test]
    fn validate_host_rejects_ipv4_literal() {
        for ip in ["192.168.1.1", "127.0.0.1", "169.254.169.254"] {
            assert!(
                matches!(
                    validate_host(ip),
                    Err(Sep1Error::MalformedHomeDomain { .. })
                ),
                "expected rejection for {ip}",
            );
        }
    }

    #[test]
    fn validate_host_rejects_ipv6_literal() {
        // The `:` makes these fail the byte-set check before IP parsing
        // even kicks in, but both gates should reject them.
        for ip in ["::1", "fe80::1"] {
            assert!(
                matches!(
                    validate_host(ip),
                    Err(Sep1Error::MalformedHomeDomain { .. })
                ),
                "expected rejection for {ip}",
            );
        }
    }

    #[test]
    fn validate_host_rejects_url_smuggling() {
        // Anything containing `/`, `:`, `@`, `?`, `#`, space, or upper
        // bytes >127 fails the alphanumeric+.+- check.
        for bad in [
            "evil.com/path",
            "evil.com:8080",
            "user@evil.com",
            "evil.com?x=y",
            "evil.com#frag",
            "evil .com",
        ] {
            assert!(
                matches!(
                    validate_host(bad),
                    Err(Sep1Error::MalformedHomeDomain { .. })
                ),
                "expected rejection for {bad}",
            );
        }
    }
}
