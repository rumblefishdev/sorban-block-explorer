---
id: '0092'
title: 'Docker Compose: full-stack environment for AI agent e2e testing'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0023', '0015']
tags: [priority-low, effort-small, layer-infra, ai-agents]
milestone: 3
links: []
history:
  - date: 2026-03-30
    status: backlog
    who: stkrolikiewicz
    note: 'Task created. AI agents cannot easily run nx serve as long-running processes — containerized full stack enables deterministic e2e testing.'
---

# Docker Compose: full-stack environment for AI agent e2e testing

## Summary

Extend `docker-compose.yml` with `web` and `api` services so AI agents (Claude Code, CI bots) can spin up the entire stack with `docker compose up -d` and run e2e tests against it. Currently only PostgreSQL is containerized; web and API require `nx serve` which is impractical for stateless agent sessions.

## Status: Backlog

**Current state:** Not started. Current docker-compose.yml has only PostgreSQL.

## Context

AI agents operate in stateless sessions and cannot easily manage long-running `nx serve` processes. A full-stack docker-compose would give them:

- `docker compose up -d` → entire stack ready in seconds
- Deterministic, isolated environment (no port conflicts, no stale state)
- `docker compose down` → clean teardown
- Reproducible across agent sessions and CI

### Current State

`docker-compose.yml` has only:
- `postgres` service (PostgreSQL 16 Alpine)

### Needed Services

- `api` — NestJS API (builds from `apps/api/`, exposes port 3000)
- `web` — Vite React app (builds from `apps/web/`, exposes port 4200)

## Implementation Plan

### Step 1: API Service

Add `api` service to docker-compose.yml with multi-stage Dockerfile (build + runtime). Depends on `postgres`, uses same env vars as `.env.example`.

### Step 2: Web Service

Add `web` service with Vite dev server or nginx serving built assets. Configure API proxy.

### Step 3: Profile Separation

Use docker compose profiles so `docker compose up` still only starts Postgres (dev default), and `docker compose --profile full up` starts everything.

## Acceptance Criteria

- [ ] `docker compose --profile full up -d` starts postgres + api + web
- [ ] `docker compose up -d` (default) still starts only postgres
- [ ] API service connects to postgres and responds on `/v1/network/stats`
- [ ] Web service serves the frontend and proxies API calls
- [ ] `docker compose down` cleanly tears down all services
- [ ] Works in CI (GitHub Actions) and in Claude Code agent sessions

## Notes

- Use docker compose profiles to avoid breaking existing dev workflow.
- This is low priority — only needed when AI agent e2e testing becomes a requirement.
- Production deployment uses CDK/Lambda/S3, not docker-compose.
