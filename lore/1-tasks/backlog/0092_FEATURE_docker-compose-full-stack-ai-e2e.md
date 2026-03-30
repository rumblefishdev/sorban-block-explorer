---
id: '0092'
title: 'Docker Compose: full-stack environment for AI agent e2e testing'
type: FEATURE
status: backlog
related_adr: []
related_tasks: ['0023', '0015', '0039']
tags: [priority-low, effort-medium, layer-infra, ai-agents, e2e, playwright]
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

Extend `docker-compose.yml` with `web` and `api` services so AI agents (Claude Code, CI bots) can spin up the entire stack with `docker compose up -d`, then run Playwright e2e tests against it. Currently only PostgreSQL is containerized; web and API require `nx serve` which is impractical for stateless agent sessions.

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

### Step 4: Playwright Setup

Add Playwright as e2e test framework:

- Install `@playwright/test` and configure in `apps/web-e2e/` (or `e2e/`)
- Configure `baseURL` to point at docker compose web service (`http://localhost:4200`)
- Add `webServer` config that runs `docker compose --profile full up -d` and waits for health checks
- Seed script for test data (minimal ledger + transaction rows in Postgres)
- Basic smoke tests: homepage loads, network stats displayed, navigation works

### Step 5: CI Gate (GitHub Actions)

Add e2e job to CI pipeline (task 0039):

- New workflow job `e2e` that runs after `build` passes
- Uses `docker compose --profile full up -d --wait` to start full stack
- Runs `npx playwright test` against containerized services
- Uploads Playwright HTML report as artifact on failure
- **Blocks merge** if e2e tests fail (required status check)

### Step 6: CLAUDE.md Documentation

Add e2e testing section to root `CLAUDE.md`:

```markdown
## E2E Testing

Before creating a PR that changes frontend or API behavior:
  npm run e2e
```

Document:
- When to run e2e (frontend changes, API changes, schema changes)
- How to run locally (`npm run e2e` or `scripts/e2e.sh`)
- How to debug failures (Playwright trace viewer, `--headed` mode)
- How to add new tests

### Step 7: Claude Code Hook

Configure `post-push` hook in `.claude/settings.json` that reminds/runs e2e:

```json
{
  "hooks": {
    "post-push": "scripts/e2e.sh"
  }
}
```

This ensures AI agents automatically run e2e after pushing. If the hook is too heavy for every push, make it a pre-PR hook or manual trigger with a CLAUDE.md reminder.

### Step 8: NPM Script Wrapper

Add convenience scripts to root `package.json`:

```json
{
  "scripts": {
    "e2e": "scripts/e2e.sh",
    "e2e:headed": "scripts/e2e.sh --headed"
  }
}
```

`scripts/e2e.sh` handles the full lifecycle:
1. `docker compose --profile full up -d --wait`
2. `npx playwright test "$@"`
3. `docker compose --profile full down`
4. Exit with Playwright's exit code

## Enforcement Strategy

Three layers ensure e2e tests are always run:

| Layer | Mechanism | Scope | Hardness |
|-------|-----------|-------|----------|
| **CI gate** | GitHub Actions required check | All PRs | Hard — blocks merge |
| **CLAUDE.md** | Convention documented for agents | Agent sessions | Soft — guidance |
| **Hook** | `post-push` or pre-PR hook | Claude Code agents | Medium — automatic reminder/run |

**CI is the hard gate** — even if an agent skips local e2e, CI catches it before merge. CLAUDE.md and hooks are acceleration layers that catch issues earlier.

## Acceptance Criteria

- [ ] `docker compose --profile full up -d` starts postgres + api + web
- [ ] `docker compose up -d` (default) still starts only postgres
- [ ] API service connects to postgres and responds on `/v1/network/stats`
- [ ] Web service serves the frontend and proxies API calls
- [ ] `docker compose down` cleanly tears down all services
- [ ] Playwright installed and configured with `baseURL` pointing to docker compose
- [ ] Seed script populates minimal test data in Postgres
- [ ] Basic smoke tests pass: homepage loads, network stats endpoint responds
- [ ] `npm run e2e` wrapper script handles full lifecycle (up → test → down)
- [ ] GitHub Actions e2e job blocks merge on failure (required status check)
- [ ] CLAUDE.md documents when/how to run e2e tests
- [ ] Claude Code hook configured for automatic e2e on push or pre-PR

## Notes

- Use docker compose profiles to avoid breaking existing dev workflow.
- This is low priority — only needed when AI agent e2e testing becomes a requirement.
- Production deployment uses CDK/Lambda/S3, not docker-compose.
- Playwright over Cypress: lighter, faster, better CI support, native `--wait-for-selector`.
- Seed data should be minimal — just enough for smoke tests, not a full dataset.
- CI gate is the most important piece — hooks and CLAUDE.md are nice-to-have acceleration.
- Hook weight: if full e2e is too slow for every push (~30s+), consider running only on branches with `feat/` or `fix/` prefix, or as a manual `npm run e2e` before `/pr`.
