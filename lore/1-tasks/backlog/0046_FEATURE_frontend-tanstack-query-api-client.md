---
id: '0046'
title: 'Frontend: TanStack Query setup, API client, polling, env config'
type: FEATURE
status: backlog
related_adr: []
related_tasks: []
tags: [priority-high, effort-small, layer-frontend-shared]
links: []
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created'
---

# Frontend: TanStack Query setup, API client, polling, env config

## Summary

Set up the TanStack Query provider, typed API client, polling configuration, and environment-based API URL injection in `apps/web/src/api/`. TanStack Query IS the browser cache for all server state -- no Redux, Zustand, or manual global cache layer is permitted.

## Status: Backlog

**Current state:** Not started.

## Context

The explorer frontend is a read-heavy SPA consuming a REST API. TanStack Query provides request de-duplication, background refetching, stale-state handling, and caching out of the box. Every page and section in the explorer fetches data through this layer.

API base URL: injected from environment variable at build time. Separate configs for dev, staging, and production. The app is deployed as a static SPA via CloudFront.

Security: no API keys in the bundle. The frontend is an anonymous public client. Protection is at the API Gateway/WAF layer.

Stale time configuration:

- Home page: 10-30 seconds + polling
- List pages: 60 seconds
- Detail pages: 5 minutes
- Search: no cache, debounced at approximately 300ms

Query key structure: `[resourceType, identifier?, { filters?, cursor? }]`

Independent section fetching: detail pages issue separate queries per section (e.g., account summary and account transactions are separate queries).

## Implementation Plan

### Step 1: API client setup

Create `apps/web/src/api/client.ts`:

- Base HTTP client (fetch or axios) with API base URL from env var
- Typed request/response helpers
- Error response parsing (extract status code, error message, error type)
- No API keys or auth headers

### Step 2: Environment configuration

Create `apps/web/src/api/config.ts`:

- Read API base URL from `import.meta.env.VITE_API_BASE_URL` (or equivalent)
- Validate URL at startup
- Dev/staging/prod environment configs via `.env` files

### Step 3: TanStack Query provider

Create `apps/web/src/api/QueryProvider.tsx`:

- Configure `QueryClient` with default options:
  - Request de-duplication enabled (default)
  - Background refetching enabled
  - Retry: 1 retry for 5xx/network errors, no retry for 4xx
  - Default stale time: 60 seconds (overridden per query)
- Wrap app root with `QueryClientProvider`

### Step 4: Query key factory

Create `apps/web/src/api/queryKeys.ts`:

- Structured query key factory per resource type
- Pattern: `[resourceType, identifier?, { filters?, cursor? }]`
- Examples:
  - `queryKeys.transactions.list({ cursor, filters })` -> `['transactions', { cursor, filters }]`
  - `queryKeys.transactions.detail(hash)` -> `['transactions', hash]`
  - `queryKeys.accounts.detail(id)` -> `['accounts', id]`
  - `queryKeys.accounts.transactions(id, { cursor })` -> `['accounts', id, 'transactions', { cursor }]`

### Step 5: Polling configuration

Create `apps/web/src/api/polling.ts`:

- Home page queries: `refetchInterval: 10000-15000` (10-15 seconds)
- List page queries: no automatic polling (refetch on window focus)
- Detail page queries: no polling (long stale time)
- Search queries: no polling, no cache (`cacheTime: 0`)

### Step 6: Typed query hooks (base patterns)

Create `apps/web/src/api/hooks/` with base hook patterns:

- `useNetworkStats()` -> GET /network/stats
- `useTransactionsList(filters, cursor)` -> GET /transactions
- `useTransactionDetail(hash)` -> GET /transactions/:hash
- Pattern established for all other resource types to follow

## Acceptance Criteria

- [ ] API client reads base URL from environment variable, no hardcoded URLs
- [ ] No API keys or auth tokens in the frontend bundle
- [ ] TanStack Query provider wraps app root with configured QueryClient
- [ ] Request de-duplication works by default (identical queries share one request)
- [ ] Background refetching enabled
- [ ] Stale times configured: Home (10-30s), Lists (60s), Detail (5min), Search (no cache)
- [ ] Query keys follow structured pattern: `[resourceType, identifier?, { filters?, cursor? }]`
- [ ] Home page queries poll at 10-15 second intervals
- [ ] Detail pages issue separate queries per section (independent section fetching)
- [ ] Environment configs exist for dev, staging, and production
- [ ] TanStack Query is the sole browser cache -- no Redux/Zustand/manual global state for server data

## Notes

- TanStack Query replaces any need for global state management for server data. Local UI state (modals, form inputs) can use React state.
- The query key factory is critical for cache invalidation predictability across the app.
- All page-specific query hooks will be implemented in their respective page tasks (0047-0059).
- The API client should integrate with the error classification utility from task 0044 for consistent error handling.
