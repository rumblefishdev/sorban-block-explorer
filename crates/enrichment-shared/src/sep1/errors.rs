//! Error type for the SEP-1 stellar.toml fetcher.
//!
//! Every variant collapses to `null` `description` / `home_page` on the
//! consumer side (warn-logged) — the API never propagates a 5xx because of
//! an enrichment failure. The variants exist so logs / metrics can attribute
//! outages to their root cause.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Sep1Error {
    /// Reserved for callers that want to express "no `home_domain` to fetch"
    /// as an error rather than short-circuiting upstream. Today's only
    /// consumer (`assets::handlers::get_asset`) skips the fetch entirely
    /// when `home_domain` is `None` / empty, so this variant is currently
    /// not constructed; kept so future consumers can map the missing-domain
    /// case onto the same enum the rest of the failures use.
    #[error("issuer has no home_domain set")]
    MissingHomeDomain,

    /// `home_domain` failed RFC 1035 validation, parsed as an IP literal,
    /// or otherwise looked unsafe to resolve. Treated like a missing
    /// domain — never followed.
    #[error("home_domain {host:?} is not a safe DNS hostname")]
    MalformedHomeDomain { host: String },

    /// Network-layer failure (DNS, TCP, TLS, HTTP).
    #[error("HTTP error fetching stellar.toml from {host}: {source}")]
    Http {
        host: String,
        #[source]
        source: reqwest::Error,
    },

    /// Either the connect timeout (1 s) or the total request budget (2 s)
    /// elapsed before the response could be parsed.
    #[error("timeout fetching stellar.toml from {host}")]
    Timeout { host: String },

    /// The remote sent more bytes than the SEP-1 100 KB cap; we reject
    /// without buffering the rest.
    #[error("stellar.toml body exceeded {limit} bytes")]
    BodyTooLarge { limit: usize },

    /// The response body was not valid UTF-8. SEP-1 mandates UTF-8 TOML.
    #[error("stellar.toml body is not valid UTF-8")]
    NonUtf8Body,

    /// TOML deserialization failed.
    #[error("malformed stellar.toml: {source}")]
    MalformedToml {
        #[source]
        source: toml::de::Error,
    },
}
