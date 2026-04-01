---
fetched: 2026-03-31
source: GitHub API (gh api repos/OWNER/REPO)
---

# Reference Projects — GitHub Metadata

## vagmi/rust-lambda

- **URL:** https://github.com/vagmi/rust-lambda
- **Description:** A template for Rust Lambda functions
- **Stack:** axum + sqlx + lambda_http + Aurora PostgreSQL Serverless v1
- **Stars:** 3 | **Forks:** 0 | **Open Issues:** 5
- **Last push:** 2024-02-01 | **Language:** TypeScript (CDKTF infra) + Rust
- **License:** Apache-2.0
- **Relevance:** Exact stack match (axum + sqlx + lambda_http + PostgreSQL). Small but demonstrates real integration patterns.
- **Note:** Last push Feb 2024 — patterns may use older crate versions but architecture is still valid.

## launchbadge/realworld-axum-sqlx

- **URL:** https://github.com/launchbadge/realworld-axum-sqlx
- **Description:** A Rust implementation of the Realworld demo app spec using Axum and SQLx
- **Stack:** axum + sqlx + PostgreSQL (no Lambda)
- **Stars:** 1,060 | **Forks:** 103 | **Open Issues:** 13
- **Last push:** 2023-12-30 | **Language:** Rust
- **License:** AGPL-3.0
- **Relevance:** Best idiomatic axum+sqlx reference at medium complexity. Built by sqlx authors (Launchbadge). Clean layered architecture.
- **Note:** Last push Dec 2023 — older axum version but patterns transferable.

## sheroz/axum-rest-api-sample

- **URL:** https://github.com/sheroz/axum-rest-api-sample
- **Description:** Building REST API Web service in Rust using axum, JWT, SQLx, PostgreSQL, and Redis
- **Stack:** axum + sqlx + PostgreSQL + Redis + JWT (no Lambda)
- **Stars:** 125 | **Forks:** 20 | **Open Issues:** 0
- **Last push:** 2026-03-23 | **Language:** Rust
- **License:** MIT
- **Relevance:** Production-quality patterns: auth, error handling, testing. Recently active.

## hanabu/lambda-web (actix-web Lambda adapter — DEAD)

- **URL:** https://github.com/hanabu/lambda-web
- **Description:** Run Rust web frameworks on AWS Lambda
- **Stack:** Adapter for actix-web/axum/rocket/warp on Lambda
- **Stars:** 120 | **Forks:** 37 | **Open Issues:** 15
- **Last push:** 2024-05-03 | **Language:** Rust
- **License:** MIT
- **Status:** Effectively unmaintained. Last crate release Jan 2023.
- **Relevance:** Archived as evidence for eliminating actix-web from consideration.

## poem-web/poem

- **URL:** https://github.com/poem-web/poem
- **Description:** A full-featured and easy-to-use web framework with the Rust programming language
- **Stack:** poem framework + poem-lambda + poem-openapi
- **Stars:** 4,376 | **Forks:** 348 | **Open Issues:** 189
- **Last push:** 2026-03-31 | **Language:** Rust
- **License:** Apache-2.0
- **Relevance:** Evaluated as strong contender. Rejected due to bus factor (83% single maintainer) and zero Lambda+PG real-world projects.
