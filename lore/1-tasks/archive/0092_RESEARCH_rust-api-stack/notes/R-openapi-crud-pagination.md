---
type: research
status: mature
spawned_from: 0092
tags: [openapi, aide, utoipa, crud, pagination, axum]
---

# OpenAPI: aide vs utoipa — CRUD, pagination, documentation

## Context

Block explorer API: ~10 resources × ~4-5 endpoints = ~50 endpoints. Need:

- Reusable CRUD base (list, get by ID)
- Cursor-based pagination (opaque base64 cursor)
- Production-quality OpenAPI documentation (descriptions, examples, tags, Swagger UI)

---

## 1. Generic CRUD Base

### utoipa: Requires `macro_rules!`

`#[utoipa::path]` is a proc macro — **cannot be applied to generic functions**. Needs concrete types at compile time.

Workaround: `macro_rules!` stamps out concrete handler functions AND concrete paginated response types per resource:

```rust
crud_routes!(
    module = ledgers,
    resource = Ledger,
    id_type = i64,
    path = "/ledgers",
    tag = "Ledgers",
    summary_list = "List ledgers",
    summary_get = "Get ledger by sequence number",
);
```

The macro generates:

- A `type PaginatedXxx = PaginatedResponse<Xxx>` alias (needed because `<>` in utoipa macro attributes causes parse errors)
- `list` and `get_by_id` handler functions with `#[utoipa::path]` annotations (using alias in `body = inline(PaginatedXxx)`)
- A `fn router()` returning the axum Router

- ~120 lines macro definition (once)
- ~10 lines per resource invocation
- **Verified pattern (PoC, cargo check ✅):** Generic `PaginatedResponse<T>` with `ToSchema` works in utoipa 5.x. Register concrete types in `#[openapi]` via turbofish: `schemas(PaginatedResponse::<Ledger>)`. Use Rust type aliases for `body = inline(PaginatedLedgers)` in handler annotations.
- **Note:** `#[aliases]` was removed in utoipa 5.x (existed in 4.x). Rust type aliases + turbofish are the replacement.

### aide: Pure Rust generics

Monomorphization gives aide concrete types at call sites. Generic handlers + `CrudRouter` builder:

```rust
CrudRouter::<Ledger, LedgerRepo>::new("/ledgers")
    .list()
    .get()
    .build()
    .with_state(Arc::new(repo))
```

- ~100 lines `CrudRouter` + traits (once)
- ~50 lines `OperationOutput` impl for custom wrappers (once)
- ~4 lines per resource wiring
- No aliases needed

**Verdict: aide wins.** Pure generics > macros for maintainability and extensibility. Adding `.create()`, `.update()`, `.delete()` = add builder method, not modify macro.

---

## 2. Cursor-Based Pagination

### utoipa

```rust
#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PaginationParams {
    /// Opaque base64-encoded cursor. Omit for first page.
    #[param(example = "eyJpZCI6MTAwfQ==")]
    pub cursor: Option<String>,

    /// Max items (1-100, default 20).
    #[param(example = 20, minimum = 1, maximum = 100)]
    pub limit: u64,
}

// Generic struct — works with utoipa 5.x
#[derive(Serialize, ToSchema)]
pub struct PaginatedResponse<T: ToSchema> {
    pub data: Vec<T>,
    #[schema(example = "eyJpZCI6MjAwfQ==", nullable)]
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

// Rust type alias — avoids <> parse issue in utoipa macro attributes
type PaginatedLedgers = PaginatedResponse<Ledger>;

// In #[utoipa::path]: use alias in body
//   responses((body = inline(PaginatedLedgers)))
// In #[openapi]: use turbofish in schemas
//   components(schemas(PaginatedResponse::<Ledger>))
// Note: components(schemas(PaginatedResponse::<Ledger>)) may be optional —
// utoipa 5.x can auto-collect schemas from path usage. Explicit registration
// useful to ensure schema presence in complex cases.
// Verified: cargo check ✅ (PoC)
// Ref: https://docs.rs/utoipa/5.4.0/utoipa/derive.ToSchema.html
```

### aide

```rust
#[derive(Deserialize, JsonSchema)]
pub struct PaginationParams {
    #[schemars(description = "Opaque base64-encoded cursor from previous response")]
    pub cursor: Option<String>,

    #[schemars(range(min = 1, max = 100))]
    pub limit: Option<i32>,
}

#[derive(Serialize, JsonSchema)]
pub struct PaginatedResponse<T: JsonSchema> {
    pub data: Vec<T>,
    #[schemars(description = "Opaque base64 cursor for next page, null if last page")]
    pub next_cursor: Option<String>,
    pub has_more: bool,
}
```

### Comparison

| Aspect                   | utoipa                                                        | aide                                                                        |
| ------------------------ | ------------------------------------------------------------- | --------------------------------------------------------------------------- |
| Per-resource boilerplate | 1 alias line per resource                                     | 0                                                                           |
| Inline param examples    | `#[param(example = ...)]` — clean                             | `#[schemars(example = ...)]` in 0.9 — clean                                 |
| Constraints              | `#[param(minimum, maximum)]`                                  | `#[schemars(range(min, max))]`                                              |
| Schema names             | Concrete types per resource (utoipa 5.x removed `#[aliases]`) | Auto: `PaginatedResponse_Ledger` (overridable via manual `JsonSchema` impl) |

**Verdict: Tie.** Both handle cursor pagination well. utoipa has cleaner schema naming out of the box. aide has zero per-resource boilerplate.

---

## 3. OpenAPI Documentation Quality

### 3.1 Field Descriptions

|                  | utoipa                                                 | aide                                                     |
| ---------------- | ------------------------------------------------------ | -------------------------------------------------------- |
| Mechanism        | `///` doc comments OR `#[schema(description = "...")]` | `///` doc comments OR `#[schemars(description = "...")]` |
| Include external | `include_str!("docs.md")`                              | Not directly (schemars limitation)                       |

**Verdict: Tie.** Both use doc comments natively.

### 3.2 Endpoint Summary & Description

|                  | utoipa                                                                             | aide                                                            |
| ---------------- | ---------------------------------------------------------------------------------- | --------------------------------------------------------------- |
| Mechanism        | `#[utoipa::path(summary = "...", description = "...")]` or doc comments on handler | `.summary("...")` / `.description("...")` on TransformOperation |
| Markdown support | Yes (inline or `include_str!`)                                                     | Yes (any string)                                                |

**Verdict: utoipa slightly better** — doc comments on handler auto-map (first line = summary, rest = description). aide requires explicit transform function.

### 3.3 Inline Examples

|                   | utoipa                                  | aide                                            |
| ----------------- | --------------------------------------- | ----------------------------------------------- |
| Schema fields     | `#[schema(example = 42)]`               | `#[schemars(example = 42)]` (0.9)               |
| Query params      | `#[param(example = "abc")]`             | `#[schemars(example = "abc")]` on struct fields |
| Response body     | `example = json!({...})` in responses() | `.example(value)` on TransformResponse          |
| Per-param in path | `("id" = i64, example = 42)`            | **No direct way** — must use `inner_mut()`      |

**Verdict: utoipa wins.** More granular control, especially for path/query parameter examples. aide's param example support is weaker.

### 3.4 Schema Naming (Generics)

|          | utoipa                                                                                        | aide                                                                   |
| -------- | --------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------- |
| Default  | `PaginatedResponse<Ledger>`                                                                   | `PaginatedResponse_Ledger`                                             |
| Override | Concrete type per resource (`#[aliases]` removed in utoipa 5.x; `#[schema(as)]` for renaming) | `#[schemars(rename = "...")]` per-concrete or manual `JsonSchema` impl |

**Verdict: Tie.** utoipa 5.x removed `#[aliases]` — both approaches need concrete types per resource. utoipa's `macro_rules!` generates these; aide's generics also need `OperationOutput` per type.

### 3.5 Swagger UI / ReDoc / Scalar

|           | utoipa                               | aide                              |
| --------- | ------------------------------------ | --------------------------------- |
| SwaggerUI | `utoipa-swagger-ui` (separate crate) | Built-in `aide::swagger::Swagger` |
| ReDoc     | `utoipa-redoc` (separate crate)      | Built-in `aide::redoc::Redoc`     |
| RapiDoc   | `utoipa-rapidoc` (separate crate)    | Not available                     |
| Scalar    | `utoipa-scalar` (separate crate)     | Built-in `aide::scalar::Scalar`   |

**Verdict: aide slightly better** — bundled behind feature flags, no extra crates. But utoipa has RapiDoc which aide doesn't.

### 3.6 Response Codes

|                  | utoipa                                                | aide                                         |
| ---------------- | ----------------------------------------------------- | -------------------------------------------- |
| Multiple codes   | `responses((status = 200, ...), (status = 404, ...))` | `.response_with::<200, T, _>(...)` chained   |
| Wildcard         | `"5XX"` supported                                     | `response_range::<5, T>()`                   |
| Default response | Via `ToResponse` trait                                | `.default_response_with::<T, _>(...)` global |

**Verdict: Tie.** Both fully capable. Different syntax, same result.

### 3.7 Deprecation

|                  | utoipa                                   | aide                                     |
| ---------------- | ---------------------------------------- | ---------------------------------------- |
| Mechanism        | `#[deprecated]` on handler (Rust native) | `inner_mut().deprecated = true` (manual) |
| On schema fields | `#[deprecated]` on struct field          | Not directly supported                   |

**Verdict: utoipa wins clearly.** Native `#[deprecated]` is zero-effort. aide's approach is a workaround.

### 3.8 Tags / Grouping

|                  | utoipa                                   | aide                                                  |
| ---------------- | ---------------------------------------- | ----------------------------------------------------- |
| Per-endpoint     | `tag = "Ledgers"` in `#[utoipa::path]`   | `.tag("Ledgers")` on TransformOperation               |
| Per-router batch | `nest()` on OpenApi with tag propagation | **Not supported** — must tag each operation           |
| Tag descriptions | In `#[openapi(tags(...))]`               | `.tag(Tag { name, description })` on TransformOpenApi |

**Verdict: utoipa wins.** Per-router tag batch is important for 10 resource modules. aide requires tagging each operation individually (workaround: helper function, but still per-op).

### 3.9 Security Schemes

Both fully capable. aide's API is slightly cleaner:

```rust
// aide
api.security_scheme("BearerAuth", SecurityScheme::Http { ... })
op.security_requirement("BearerAuth")

// utoipa
impl Modify for SecurityAddon { fn modify(&self, openapi: &mut OpenApi) { ... } }
#[openapi(modifiers(&SecurityAddon), security(("BearerAuth" = [])))]
```

**Verdict: aide slightly better** — less boilerplate for common case.

### 3.10 Enum Documentation

|              | utoipa                           | aide (schemars)                  |
| ------------ | -------------------------------- | -------------------------------- |
| Simple enums | `enum: ["A", "B"]`               | `enum: ["A", "B"]`               |
| Mixed enums  | `oneOf` with per-variant schemas | `oneOf` with per-variant schemas |
| Serde tagged | Respects `#[serde(tag = "...")]` | Respects `#[serde(tag = "...")]` |

**Verdict: Tie.** Both respect serde attributes.

---

## 4. Summary Scorecard

| Category               | utoipa                                                    | aide                         | Winner     |
| ---------------------- | --------------------------------------------------------- | ---------------------------- | ---------- |
| **CRUD base pattern**  | Requires `macro_rules!`                                   | Pure Rust generics           | **aide**   |
| **Cursor pagination**  | Aliases needed, clean names                               | Zero boilerplate, ugly names | Tie        |
| **Field descriptions** | Doc comments                                              | Doc comments                 | Tie        |
| **Endpoint docs**      | Doc comments auto-map                                     | Explicit transform fn        | **utoipa** |
| **Inline examples**    | Everywhere (fields, params, responses)                    | Weak on params               | **utoipa** |
| **Schema naming**      | Concrete types per resource (`#[aliases]` removed in 5.x) | Manual `JsonSchema` impl     | Tie        |
| **Doc UIs**            | 4 options (separate crates)                               | 3 built-in                   | Tie        |
| **Response codes**     | Rich syntax                                               | Rich API                     | Tie        |
| **Deprecation**        | `#[deprecated]` native                                    | Manual `inner_mut()`         | **utoipa** |
| **Tags**               | Per-router batch via `nest()`                             | Per-operation only           | **utoipa** |
| **Security**           | `Modify` trait (verbose)                                  | `.security_scheme()` (clean) | **aide**   |
| **Enums**              | Full serde compat                                         | Full serde compat            | Tie        |

**Score: utoipa 4, aide 2, tie 6.** (Schema naming changed from utoipa win to tie after `#[aliases]` removal confirmed)

---

## 5. Revised Recommendation

For a CRUD-heavy API with 10 resources requiring production-quality OpenAPI documentation:

**utoipa 5.4 is the better choice.**

The CRUD generic pattern (aide's main advantage) can be solved with a `macro_rules!` in utoipa — not as elegant as pure generics, but proven and cargo-check verified.

The documentation gaps in aide are real and harder to work around:

- No `#[deprecated]` support
- No per-router tag batch
- Weaker parameter examples
- Ugly generic schema names without manual impl

These matter for a public API with 50+ endpoints consumed by frontend developers who rely on Swagger UI.

**Trade-off accepted:** utoipa's `#[utoipa::path]` annotations are more verbose per handler, but the macro approach keeps per-resource overhead to ~10 lines. The total boilerplate difference is ~0 lines (both ~950 for 10 resources).

---

## Sources

- utoipa 5.4 docs: https://docs.rs/utoipa/5.4.0
- utoipa-axum 0.2 docs: https://docs.rs/utoipa-axum/0.2.0
- aide 0.15 docs: https://docs.rs/aide/0.15.1
- schemars 0.9 docs: https://docs.rs/schemars/0.9.0 (transitional — current stable is 1.2.1)
- schemars 1.2 docs: https://docs.rs/schemars/1.2.1
- Note: aide 0.15 locked to schemars 0.9; aide 0.16 (alpha) supports schemars 1.x
- TransformOperation API: https://docs.rs/aide/0.15.1/aide/transform/struct.TransformOperation.html
