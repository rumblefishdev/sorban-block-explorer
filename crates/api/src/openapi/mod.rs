//! OpenAPI specification root document.
//!
//! This module defines [`ApiDoc`] — the metadata-only root document
//! (title, version, description, shared schema components). Endpoint
//! paths are registered dynamically at runtime through
//! `utoipa_axum::router::OpenApiRouter::routes`, and the runtime
//! `servers` block is stamped onto the registered spec by `main::app`
//! so the advertised base URL can vary per deployment environment.

pub mod schemas;

use utoipa::OpenApi;

use schemas::{ErrorEnvelope, PageInfo};

/// Root OpenAPI document. Holds API metadata and declares shared
/// schema components that are referenced across multiple endpoints.
///
/// Paths are registered dynamically through `OpenApiRouter::routes`
/// so handler modules don't need to be listed here explicitly — M2
/// endpoint modules add routes without touching this file.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Soroban Block Explorer API",
        version = env!("CARGO_PKG_VERSION"),
        description = "REST API exposing ledger, transaction, contract, and NFT \
                       data for the Soroban block explorer. All list endpoints \
                       use cursor-based pagination (see ADR 0008).",
        contact(name = "Rumble Fish", url = "https://rumblefish.dev"),
        license(name = "Proprietary"),
    ),
    components(schemas(ErrorEnvelope, PageInfo)),
)]
pub struct ApiDoc;
