---
id: '0106'
title: 'BUG: SPA index.html cached for 24h on CloudFront'
type: BUG
status: completed
related_adr: []
related_tasks: ['0035']
tags: [priority-high, effort-small, layer-infra]
links: []
history:
  - date: 2026-04-07
    status: active
    who: stkrolikiewicz
    note: >
      Spawned from 0035 adversarial review (P6). Default behavior on the
      SPA CloudFront distribution uses CACHING_OPTIMIZED which has a 1-day
      default TTL. That means index.html is cached at the edge for 24h
      after each frontend deploy — users with a cache hit see stale
      bundle references for up to a day. Hot-fix before frontend pipeline
      (task 0039) goes live.
  - date: 2026-04-07
    status: completed
    who: stkrolikiewicz
    note: >
      Added IndexHtmlCachePolicy (default 60s, max 5min) and an
      additionalBehaviors entry for /index.html. Default behavior keeps
      CACHING_OPTIMIZED for hashed assets. Verified in synthesized
      template: /index.html uses the new policy ref while the default
      behavior keeps the managed policy ID. Shared behavior props
      extracted into a local helper to avoid duplication of origin /
      headers / function association across both behaviors.
---

# BUG: SPA index.html cached for 24h on CloudFront

## Summary

`DeliveryStack` configures `cachePolicy: cloudfront.CachePolicy.CACHING_OPTIMIZED` on the default behavior. This managed policy has a default TTL of 1 day. As a result, after a frontend deploy:

- Hashed asset files (`main.abc123.js`, etc.) — fine, content-addressed, can be cached forever
- **`index.html`** — references the hashed assets and **must** be revalidated frequently. With 24h cache, users with a cache hit get the OLD `index.html` pointing at NEW (or missing) bundle paths

This is a **deal-breaker for SPA deploys** once task 0039 (CI/CD frontend pipeline) goes live. Caught in adversarial review of 0035 PR #69 after the merge.

## Root Cause

The original 0035 spec said:

> Cache behavior: long TTL for static assets (JS, CSS, images with content hash), **short TTL for `index.html`**

The implementation collapsed both into a single `defaultBehavior` with `CACHING_OPTIMIZED`, ignoring the spec's split. SPA fallback via `errorResponses` (403/404 → /index.html with TTL 0) handles the _fallback_ case but not direct requests to `/` or `/index.html`.

## Fix

Add an `additionalBehaviors` entry for `/index.html` with a custom `CachePolicy` that has a short TTL (e.g., 60s default, 5min max). The default behavior (long TTL via `CACHING_OPTIMIZED`) continues to handle hashed assets.

CloudFront `defaultRootObject` rewrites `/` → `/index.html` before behavior matching, so the new behavior covers both apex requests and explicit `/index.html` requests.

## Acceptance Criteria

- [x] `index.html` cache TTL is at most 5 minutes at the edge (60s default, 300s max)
- [x] Hashed assets (default behavior) keep long TTL (CACHING_OPTIMIZED)
- [x] CloudFront Function (basic auth) attaches to BOTH default and index.html behaviors when `enableBasicAuth` is true (via `sharedBehaviorProps`)
- [x] Response headers policy attaches to BOTH behaviors (via `sharedBehaviorProps`)
- [x] Verified in synthesized template — IndexHtmlCachePolicy ref on /index.html behavior, managed policy ID on default behavior

## Notes

- This is a CDK-only fix; no SPA build changes needed
- The fix is independent of task 0039 (CI/CD) — they can land in either order
- Spawned from 0035 adversarial review point P6
