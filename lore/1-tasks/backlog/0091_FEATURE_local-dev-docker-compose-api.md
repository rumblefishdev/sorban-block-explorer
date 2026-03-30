---
id: '0091'
title: 'Local dev: NestJS API local entrypoint and dev workflow'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0023', '0016', '0045', '0038']
tags: [priority-medium, effort-small, layer-backend]
milestone: 2
links: []
history:
  - date: 2026-03-30
    status: backlog
    who: FilipDz
    note: 'Spawned from 0023 discussion - team needs local dev workflow for NestJS API'
---

# Local dev: NestJS API local entrypoint and dev workflow

## Summary

Create a local development entrypoint for the NestJS API so the team can run and test the API locally against the docker-compose PostgreSQL. The current `main.ts` is a Lambda handler (serverless-express) and cannot serve HTTP directly — a separate `main.local.ts` with `app.listen()` is needed.

## Context

`docker-compose.yml` already provides PostgreSQL. The API needs a local entrypoint that reuses `AppModule` but bootstraps with `app.listen()` instead of the Lambda wrapper. The primary dev workflow should be **API running natively** (via Nx serve target with hot reload), connecting to PostgreSQL in docker-compose. This gives fast rebuild cycles and standard NestJS DX.

Optionally, an API service can be added to docker-compose for CI integration tests or full-stack demo, but this is not the primary dev workflow.

Best done after DB schema (0016) and a first data-serving module (e.g. 0045 network) so there's something to test end-to-end.

## Implementation Plan

1. Create `apps/api/src/main.local.ts` — NestJS bootstrap with `app.listen()`, reusing `AppModule`, with global prefix and exception filter (same as Lambda handler)
2. Add Nx `serve` target for `apps/api` so `nx serve api` runs the local entrypoint with hot reload
3. Create `.env.example` with local DB credentials matching docker-compose defaults (`RDS_PROXY_HOST=localhost`, `DB_NAME=soroban_block_explorer`, `DB_USER=postgres`, `DB_PASSWORD=postgres`, `DB_PORT=5432`)
4. (Optional) Add `api` service to `docker-compose.yml` for integration testing / full-stack demo mode
5. Document local dev setup in README (docker-compose up for DB, nx serve api for API)

## Acceptance Criteria

- [ ] `main.local.ts` starts NestJS with `app.listen()` on configurable port
- [ ] `nx serve api` runs the local entrypoint with hot reload
- [ ] API connects to docker-compose PostgreSQL and serves `GET /v1` health endpoint
- [ ] `.env.example` documents all required env vars with local defaults
- [ ] Local dev workflow documented in README

## Notes

- `main.local.ts` and `main.ts` (Lambda) share the same `AppModule` — no code duplication
- Running API natively (not in container) is preferred for dev because of hot reload and faster feedback
- Docker-compose API service is optional and secondary — useful for CI or onboarding demos
- ConfigModule.forRoot() already reads `.env` files by default
