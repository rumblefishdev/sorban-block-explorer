---
id: '0042'
title: 'OpenAPI/Swagger infrastructure setup'
type: FEATURE
status: backlog
related_adr: ['0005']
related_tasks: ['0023', '0057', '0092']
tags: [priority-medium, effort-small, layer-backend]
milestone: 1
links: []
history:
  - date: 2026-03-30
    status: backlog
    who: fmazur
    note: 'Task created — split from 0057 during milestone alignment (task 0085). D1 requires OpenAPI specification infrastructure; full endpoint documentation is M2 (task 0057).'
  - date: 2026-03-31
    status: backlog
    who: stkrolikiewicz
    note: 'Updated per ADR 0005: NestJS → Rust (axum + utoipa + sqlx)'
---

# OpenAPI/Swagger infrastructure setup

## Summary

Set up utoipa OpenAPI integration, document builder configuration, utoipa-swagger-ui dev endpoint, and spec export pipeline. This is the M1 infrastructure prerequisite for the full OpenAPI endpoint documentation (task 0057, M2). D1 design scope includes "OpenAPI specification" — this task delivers the tooling and empty spec skeleton; task 0057 fills it with all 20+ endpoint annotations.

> **Stack:** axum 0.8 + utoipa 5.4 + sqlx 0.8 (per ADR 0005). Code in crates/api/.

## Status: Backlog

**Current state:** Not started. Depends on task 0023 (API bootstrap).

## Context

The technical design (§7.4 D1) lists "OpenAPI specification" in the D1 scope. However, the full spec (task 0057) requires all API feature modules (M2) to exist. This task splits out the infrastructure part that can be delivered in M1: utoipa setup, document builder, dev UI, and export pipeline.

## Implementation Plan

### Step 1: Install and configure utoipa + utoipa-swagger-ui

Add `utoipa` and `utoipa-swagger-ui` dependencies. Configure `OpenApi` derive macro with API title, description, version, base URL, and contact info in `crates/api/src/`.

### Step 2: Define reusable schema components

Create shared OpenAPI schema components (via `ToSchema` derive) for: error envelope, pagination envelope, standard query parameters. These components will be referenced by endpoint annotations in task 0057.

### Step 3: Swagger UI dev endpoint

Configure utoipa-swagger-ui at `/api-docs` in development/staging environments. Ensure it is disabled in production.

### Step 4: Spec export as JSON

Set up OpenAPI spec export as JSON at `/api-docs-json` via axum route.

## Acceptance Criteria

- [ ] `utoipa` and `utoipa-swagger-ui` configured in the API crate
- [ ] OpenApi derive configured with API metadata (title, version, description)
- [ ] Reusable schema components defined via `ToSchema` (error envelope, pagination envelope)
- [ ] Swagger UI available at `/api-docs` in dev/staging
- [ ] OpenAPI spec exportable as JSON at `/api-docs-json`
- [ ] Swagger UI and spec JSON served directly from the API (no S3 publication pipeline)

## Notes

- This task delivers the "OpenAPI specification" infrastructure required by D1.
- Task 0057 (M2) depends on this and adds full endpoint annotations after all API modules are built.
