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
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::assets::dto::{AssetDetailResponse, AssetItem, AssetTransactionItem};
use crate::contracts::dto::{
    ContractDetailResponse, ContractStats, EventItem, InterfaceResponse, InvocationItem,
};
use crate::stellar_archive::dto::{
    E3HeavyFields, E3Response, HeavyFieldsStatus, SignatureDto, XdrEventDto, XdrOperationDto,
};
use crate::transactions::dto::{
    EventAppearanceItem, InvocationAppearanceItem, OperationItem, TransactionDetailLight,
    TransactionListItem,
};
use schemas::{ErrorEnvelope, PageInfo, Paginated};

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
    components(schemas(
        ErrorEnvelope,
        PageInfo,
        Paginated<TransactionListItem>,
        TransactionListItem,
        E3Response<TransactionDetailLight>,
        TransactionDetailLight,
        OperationItem,
        EventAppearanceItem,
        InvocationAppearanceItem,
        E3HeavyFields,
        SignatureDto,
        XdrEventDto,
        XdrOperationDto,
        HeavyFieldsStatus,
        ContractDetailResponse,
        ContractStats,
        InterfaceResponse,
        Paginated<InvocationItem>,
        InvocationItem,
        Paginated<EventItem>,
        EventItem,
        Paginated<AssetItem>,
        AssetItem,
        AssetDetailResponse,
        Paginated<AssetTransactionItem>,
        AssetTransactionItem,
    )),
)]
pub struct ApiDoc;

/// Build the `OpenApiRouter` carrying every endpoint advertised by the
/// API. Shared between the live Lambda app (`main::app`) and the
/// build-time `extract_openapi` binary so neither can quietly drop a
/// route from the spec the frontend codegen consumes.
///
/// Returns a router typed with `AppState` but not yet bound to a state
/// value. Callers attach state via `.with_state(...)` (live app) or
/// call `.split_for_parts()` directly (extractor — the spec does not
/// depend on `AppState`).
pub fn register_routes() -> OpenApiRouter<crate::AppState> {
    OpenApiRouter::with_openapi(ApiDoc::openapi())
        .routes(routes!(crate::ops::health))
        .nest("/v1", crate::network::router())
        .nest("/v1", crate::transactions::router())
        .nest("/v1", crate::contracts::router())
        .nest("/v1", crate::liquidity_pools::router())
        .nest("/v1", crate::assets::router())
        .nest("/v1", crate::ledgers::router())
}
