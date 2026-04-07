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
      Added ShortTtlCachePolicy (default 60s, max 5min) and applied it
      to the default CloudFront behavior — covers index.html, the apex,
      and any unknown path (safe by default). Long TTL via the managed
      CACHING_OPTIMIZED policy is now opt-in via additionalBehaviors for
      `/assets/*` (Vite default) and `/static/*` (CRA default). First
      iteration kept CACHING_OPTIMIZED on the default and put short TTL
      on an additionalBehaviors[/index.html] entry, but that relied on
      an unverified assumption about CloudFront defaultRootObject
      rewriting and behavior matching order — inverted the strategy in
      a follow-up commit for safety. Shared behavior props extracted
      into a local helper. Verified in synthesized template: default
      behavior references ShortTtlCachePolicy, /assets/* and /static/*
      reference the managed CACHING_OPTIMIZED ID.
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

Inverted cache strategy — **safe by default**:

- **Default behavior** uses a new `ShortTtlCachePolicy` (60s default, 5min max). This covers `index.html`, the apex `/`, and any unknown path. Any SPA build artifact that doesn't match the explicit asset patterns falls through here and gets the safe short TTL (acceptable degradation, not a bug).
- **`additionalBehaviors`** opt-in long TTL via the AWS-managed `CACHING_OPTIMIZED` policy (1 day default, 1 year max) for known hashed-asset directories: `/assets/*` (Vite default output) and `/static/*` (Create React App default output).
- All behaviors share origin / response headers policy / basic auth function via a local `sharedBehaviorProps` helper, so the security gating cannot drift between cache behaviors.

The first iteration tried the opposite (long TTL on default, short TTL on `additionalBehaviors[/index.html]`), but that depended on an unverified assumption that CloudFront's `defaultRootObject` rewriting happens **before** behavior matching. Inverting eliminates the assumption — the apex request hits the safe default regardless of when (or if) the rewrite happens.

## Acceptance Criteria

- [x] `index.html` cache TTL is at most 5 minutes at the edge (default behavior, 60s default, 300s max)
- [x] Hashed assets keep long TTL (`/assets/*` and `/static/*` use managed `CACHING_OPTIMIZED`)
- [x] CloudFront Function (basic auth) attaches to ALL behaviors when `enableBasicAuth` is true (via `sharedBehaviorProps`)
- [x] Response headers policy attaches to ALL behaviors (via `sharedBehaviorProps`)
- [x] Verified in synthesized template — default behavior references `ShortTtlCachePolicy`, `/assets/*` and `/static/*` reference the managed `CACHING_OPTIMIZED` ID

## Notes

- This is a CDK-only fix; no SPA build changes needed
- The fix is independent of task 0039 (CI/CD) — they can land in either order
- Spawned from 0035 adversarial review point P6
