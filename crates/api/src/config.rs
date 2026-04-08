//! Runtime configuration for the API service.
//!
//! All environment variable reads happen in [`AppConfig::from_env`] so
//! that `fn app(&AppConfig) -> Router` stays pure — tests construct
//! their own `AppConfig` without touching `std::env`.

/// Application-wide runtime configuration.
///
/// The `version` advertised in the OpenAPI spec is sourced from
/// `env!("CARGO_PKG_VERSION")` directly at the `ApiDoc` derive site,
/// so it does not need to live on this struct.
#[derive(Debug, Clone)]
pub struct AppConfig {
    /// Fully-qualified base URL advertised to OpenAPI clients in the
    /// `servers` block. In production this is the API Gateway custom
    /// domain (e.g. `https://api.staging.sorobanscan.rumblefish.dev`);
    /// locally it falls back to `http://localhost:9000`.
    pub base_url: String,
}

impl AppConfig {
    /// Build an `AppConfig` from the process environment.
    ///
    /// Reads:
    /// - `API_BASE_URL` — set by CDK (`compute-stack.ts`) from the
    ///   environment's `apiDomainName`. Falls back to a local default
    ///   when unset so `cargo run -p api` works out of the box.
    pub fn from_env() -> Self {
        Self {
            base_url: std::env::var("API_BASE_URL")
                .unwrap_or_else(|_| "http://localhost:9000".to_string()),
        }
    }
}
