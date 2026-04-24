//! Opt-in CRUD scaffolding for simple list/detail endpoints.
//!
//! A resource that can be served by a pair of straight "select-by-cursor"
//! and "select-by-id" queries (no post-fetch XDR enrichment, no join
//! gymnastics, no per-endpoint filter DSL) implements [`CrudResource`]
//! and invokes [`crud_routes!`] at its module root. The macro wires up
//! the two `GET` handlers, the `#[utoipa::path]` annotations, and the
//! `OpenApiRouter` — the resource module contributes only the SQL.
//!
//! Resources with custom behaviour (e.g. transactions — memo enrichment
//! via S3 XDR fetch, StrKey shape validation per filter key, dynamic
//! JOIN selection) do **not** use this trait. They consume the low-level
//! helpers in [`super::cursor`], [`super::pagination`], [`super::errors`],
//! [`super::extractors`], [`super::filters`] directly. See the 2026-04-24
//! Emerged design note in task 0043 — rule of three was not met at the
//! time this scaffold landed, so the trait currently has no consumer and
//! exists as infrastructure for the first resource that *does* fit the
//! simple mould (candidates: ledgers, accounts).

use std::future::Future;

use axum::Json;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde::de::DeserializeOwned;
use utoipa::ToSchema;

use super::cursor::TsIdCursor;
use super::errors;
use super::extractors::Pagination;
use super::pagination::{finalize_ts_id_page, into_envelope};

/// Contract implemented by a resource module to participate in
/// [`crud_routes!`]-generated routing.
///
/// Associated types split responsibilities cleanly:
///
/// * `State` — application state passed through axum; usually the
///   project-wide `AppState`, but the trait does not assume that so
///   test modules can supply lighter stand-ins.
/// * `Id` — path parameter type for the detail endpoint, decoded by
///   axum's [`Path`] extractor.
/// * `Row` — DB-side struct (typically `sqlx::FromRow` or hand-mapped).
/// * `Item` — wire-facing response DTO. The `ToSchema` bound is what
///   lets utoipa pick up the response body type at compile time.
///
/// Both list and detail queries are async methods returning
/// `impl Future + Send` so the generated handlers compose freely inside
/// axum's `tower::Service` machinery.
// Trait ships with zero current consumers (see task 0043 → Emerged decisions).
// `#[allow(dead_code)]` covers the trait itself and its generated handler
// bodies below until the first simple resource (candidates: ledgers,
// accounts) implements it.
#[allow(dead_code)]
pub trait CrudResource: Send + Sync + 'static {
    /// Shared application state handed to every query method.
    type State: Clone + Send + Sync + 'static;

    /// Path parameter for the detail endpoint.
    type Id: DeserializeOwned + Send + Sync + 'static;

    /// DB row type. Not exposed on the wire — [`CrudResource::into_item`]
    /// projects it onto [`CrudResource::Item`] before serialisation.
    type Row: Send + Sync + 'static;

    /// Response DTO for both list and detail endpoints.
    type Item: ToSchema + Serialize + Send + Sync + 'static;

    /// Fetch one row by its primary key, or `None` when absent.
    fn get_one(
        state: &Self::State,
        id: Self::Id,
    ) -> impl Future<Output = Result<Option<Self::Row>, sqlx::Error>> + Send;

    /// Fetch up to `limit + 1` rows for a page.
    ///
    /// The `+ 1` is load-bearing — [`finalize_ts_id_page`] uses the peek
    /// row to set `has_more` without a separate `COUNT(*)` query.
    fn get_list(
        state: &Self::State,
        cursor: Option<TsIdCursor>,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<Self::Row>, sqlx::Error>> + Send;

    /// Project a row onto the public response shape.
    fn into_item(row: Self::Row) -> Self::Item;

    /// Extract the `(created_at, id)` ordering key from a row.
    ///
    /// Called at most once per page (for the last row kept) by
    /// [`finalize_ts_id_page`]. Resources with a different natural
    /// ordering (e.g. sequence number) are not a fit for this trait —
    /// they define a bespoke cursor payload and use the low-level
    /// helpers in [`super::pagination`] directly.
    fn cursor_of(row: &Self::Row) -> TsIdCursor;
}

// ---------------------------------------------------------------------------
// Shared handler bodies. The macro wraps these with `#[utoipa::path]`
// annotations; the bodies themselves are plain generic async functions
// so the macro does not inline them into every resource's object file.
// ---------------------------------------------------------------------------

/// Body of the generated `GET /` list handler.
#[allow(dead_code)]
pub async fn list_handler<R: CrudResource>(
    State(state): State<R::State>,
    pagination: Pagination<TsIdCursor>,
) -> Response {
    let limit = pagination.limit;
    let mut rows: Vec<R::Row> = match R::get_list(&state, pagination.cursor, limit).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!("DB error in CrudResource::get_list: {e}");
            return errors::internal_error(errors::DB_ERROR, "database error");
        }
    };

    let page = finalize_ts_id_page(
        &mut rows,
        limit,
        |r| R::cursor_of(r).ts,
        |r| R::cursor_of(r).id,
    );
    let data: Vec<R::Item> = rows.into_iter().map(R::into_item).collect();
    Json(into_envelope(data, page)).into_response()
}

/// Body of the generated `GET /{id}` detail handler.
#[allow(dead_code)]
pub async fn detail_handler<R: CrudResource>(
    State(state): State<R::State>,
    Path(id): Path<R::Id>,
) -> Response {
    match R::get_one(&state, id).await {
        Ok(Some(row)) => Json(R::into_item(row)).into_response(),
        Ok(None) => errors::not_found("resource not found"),
        Err(e) => {
            tracing::error!("DB error in CrudResource::get_one: {e}");
            errors::internal_error(errors::DB_ERROR, "database error")
        }
    }
}

// ---------------------------------------------------------------------------
// crud_routes! macro
// ---------------------------------------------------------------------------

/// Generate a pair of `GET /` + `GET /{id}` axum handlers and an
/// `OpenApiRouter` for a [`CrudResource`] implementor.
///
/// `#[utoipa::path]` attribute macros require a literal string for
/// `path` and `tag`, which is why these are passed to the macro at the
/// call site rather than read off the trait as associated consts —
/// `utoipa`'s proc macro sees the literal directly.
///
/// Example shape (at the resource module root):
///
/// ```ignore
/// crate::common::crud::crud_routes!(
///     resource: LedgersResource,
///     path: "/ledgers",
///     id_path: "/ledgers/{seq}",
///     tag: "ledgers",
///     state: crate::state::AppState,
///     item: LedgerListItem,
/// );
/// ```
///
/// The macro expands to two `#[utoipa::path]`-annotated wrapper handlers
/// plus a `pub fn router() -> OpenApiRouter<$state>` that registers both.
#[macro_export]
macro_rules! crud_routes {
    (
        resource: $resource:ty,
        path: $path:literal,
        id_path: $id_path:literal,
        tag: $tag:literal,
        state: $state:ty,
        item: $item:ty $(,)?
    ) => {
        // utoipa's `body = ...` proc macro treats the last path segment as
        // the type name in generated `ToSchema` wiring, so a fully-qualified
        // `$crate::openapi::schemas::Paginated<...>` does not resolve. Pull
        // the envelope types into scope via a `use` emitted by the macro.
        use $crate::openapi::schemas::{ErrorEnvelope, Paginated};

        #[::utoipa::path(
                            get,
                            path = $path,
                            tag = $tag,
                            responses(
                                (status = 200, description = "Paginated list",
                                 body = Paginated<$item>),
                                (status = 400, description = "Invalid query parameter",
                                 body = ErrorEnvelope),
                                (status = 500, description = "Internal server error",
                                 body = ErrorEnvelope),
                            ),
                        )]
        pub async fn list(
            state: ::axum::extract::State<$state>,
            pagination: $crate::common::extractors::Pagination<$crate::common::cursor::TsIdCursor>,
        ) -> ::axum::response::Response {
            $crate::common::crud::list_handler::<$resource>(state, pagination).await
        }

        #[::utoipa::path(
                            get,
                            path = $id_path,
                            tag = $tag,
                            responses(
                                (status = 200, description = "Resource detail", body = $item),
                                (status = 404, description = "Not found",
                                 body = ErrorEnvelope),
                                (status = 500, description = "Internal server error",
                                 body = ErrorEnvelope),
                            ),
                        )]
        pub async fn detail(
            state: ::axum::extract::State<$state>,
            id: ::axum::extract::Path<<$resource as $crate::common::crud::CrudResource>::Id>,
        ) -> ::axum::response::Response {
            $crate::common::crud::detail_handler::<$resource>(state, id).await
        }

        pub fn router() -> ::utoipa_axum::router::OpenApiRouter<$state> {
            ::utoipa_axum::router::OpenApiRouter::new()
                .routes(::utoipa_axum::routes!(list))
                .routes(::utoipa_axum::routes!(detail))
        }
    };
}

#[cfg(test)]
mod tests {
    //! Compile-time + runtime smoke test for the trait and macro.
    //!
    //! Defines a toy in-memory resource to prove the trait bounds add up
    //! and the generated router responds correctly without touching
    //! Postgres. Does **not** exercise sqlx — that belongs to the
    //! integration tests added under `tests/` in Step 7.

    // `crud_routes!` emits `pub fn router()` (and `pub async fn list`/
    // `detail`) that reference the `WidgetResource` / `WidgetState` types
    // defined in this private test module, which rustc flags as
    // `private_interfaces`. Silence here — the items are never callable
    // outside the test module; the macro shape is correct for real
    // production use sites where the state type is public.
    #![allow(private_interfaces)]

    use super::*;
    use crate::common::cursor::TsIdCursor;
    use axum::body::{self, Body};
    use axum::http::{Request, StatusCode};
    use chrono::{DateTime, TimeZone, Utc};
    use serde::Deserialize;
    use std::sync::Arc;
    use tower::ServiceExt;

    #[derive(Debug, Clone)]
    struct Widget {
        id: i64,
        ts: DateTime<Utc>,
        name: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
    struct WidgetItem {
        id: i64,
        name: String,
    }

    #[derive(Clone)]
    struct WidgetState {
        rows: Arc<Vec<Widget>>,
    }

    struct WidgetResource;

    impl CrudResource for WidgetResource {
        type State = WidgetState;
        type Id = i64;
        type Row = Widget;
        type Item = WidgetItem;

        async fn get_one(
            state: &Self::State,
            id: Self::Id,
        ) -> Result<Option<Self::Row>, sqlx::Error> {
            Ok(state.rows.iter().find(|w| w.id == id).cloned())
        }

        async fn get_list(
            state: &Self::State,
            cursor: Option<TsIdCursor>,
            limit: u32,
        ) -> Result<Vec<Self::Row>, sqlx::Error> {
            let mut rows: Vec<Widget> = state.rows.iter().cloned().collect();
            rows.sort_by(|a, b| b.ts.cmp(&a.ts).then(b.id.cmp(&a.id)));
            if let Some(c) = cursor {
                rows.retain(|r| (r.ts, r.id) < (c.ts, c.id));
            }
            rows.truncate((limit + 1) as usize);
            Ok(rows)
        }

        fn into_item(row: Self::Row) -> Self::Item {
            WidgetItem {
                id: row.id,
                name: row.name,
            }
        }

        fn cursor_of(row: &Self::Row) -> TsIdCursor {
            TsIdCursor::new(row.ts, row.id)
        }
    }

    // Invoke the macro at module scope so it lives alongside the resource
    // impl exactly as a real resource module would be laid out.
    crate::crud_routes!(
        resource: WidgetResource,
        path: "/widgets",
        id_path: "/widgets/{id}",
        tag: "widgets",
        state: WidgetState,
        item: WidgetItem,
    );

    fn state() -> WidgetState {
        let rows = (1..=7)
            .map(|i| Widget {
                id: i,
                ts: Utc.with_ymd_and_hms(2026, 4, 24, 12, 0, i as u32).unwrap(),
                name: format!("w{i}"),
            })
            .collect();
        WidgetState {
            rows: Arc::new(rows),
        }
    }

    async fn body_json(resp: axum::response::Response) -> serde_json::Value {
        let bytes = body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn list_paginates_and_exposes_cursor() {
        use utoipa_axum::router::OpenApiRouter;
        let (app, _spec) = OpenApiRouter::<WidgetState>::new()
            .merge(router())
            .with_state(state())
            .split_for_parts();

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/widgets?limit=3")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let json = body_json(resp).await;
        assert_eq!(json["data"].as_array().unwrap().len(), 3);
        assert_eq!(json["page"]["limit"], 3);
        assert_eq!(json["page"]["has_more"], true);
        assert!(json["page"]["cursor"].is_string());
    }

    #[tokio::test]
    async fn list_last_page_has_null_cursor() {
        use utoipa_axum::router::OpenApiRouter;
        let (app, _spec) = OpenApiRouter::<WidgetState>::new()
            .merge(router())
            .with_state(state())
            .split_for_parts();

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/widgets?limit=50")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let json = body_json(resp).await;
        assert_eq!(json["data"].as_array().unwrap().len(), 7);
        assert_eq!(json["page"]["has_more"], false);
        // `cursor` is serialised with `skip_serializing_if = "Option::is_none"`,
        // so its absence on the last page is the correct wire shape.
        assert!(json["page"].get("cursor").is_none());
    }

    #[tokio::test]
    async fn detail_returns_row() {
        use utoipa_axum::router::OpenApiRouter;
        let (app, _spec) = OpenApiRouter::<WidgetState>::new()
            .merge(router())
            .with_state(state())
            .split_for_parts();

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/widgets/3")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["id"], 3);
        assert_eq!(json["name"], "w3");
    }

    #[tokio::test]
    async fn detail_missing_returns_404_envelope() {
        use utoipa_axum::router::OpenApiRouter;
        let (app, _spec) = OpenApiRouter::<WidgetState>::new()
            .merge(router())
            .with_state(state())
            .split_for_parts();

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/widgets/999")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let json = body_json(resp).await;
        assert_eq!(json["code"], "not_found");
    }

    #[tokio::test]
    async fn list_invalid_limit_returns_400_envelope() {
        use utoipa_axum::router::OpenApiRouter;
        let (app, _spec) = OpenApiRouter::<WidgetState>::new()
            .merge(router())
            .with_state(state())
            .split_for_parts();

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/widgets?limit=999")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let json = body_json(resp).await;
        assert_eq!(json["code"], "invalid_limit");
    }
}
