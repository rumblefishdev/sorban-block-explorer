---
id: '0107'
title: 'Fix API custom domain DNS resolution'
type: BUG
status: completed
related_adr: []
related_tasks: ['0035']
tags: [priority-high, effort-small, layer-infra]
milestone: 1
links: []
history:
  - date: 2026-04-07
    status: backlog
    who: fmazur
    note: 'Discovered during task 0039 staging deploy verification.'
  - date: 2026-04-07
    status: active
    who: fmazur
    note: 'Activated task'
  - date: 2026-04-07
    status: completed
    who: fmazur
    note: >
      Root cause: staging config pointed to parent hosted zone
      (sorobanscan.rumblefish.dev) but NS delegation sends queries
      to child zone (staging.sorobanscan.rumblefish.dev). Records
      created in parent were invisible to DNS. Fix: updated
      hostedZoneId and hostedZoneName to child zone.
---

# Fix API custom domain DNS resolution

## Summary

`api.staging.sorobanscan.rumblefish.dev` did not resolve publicly despite Route 53 records existing.

## Acceptance Criteria

- [x] `curl https://api.staging.sorobanscan.rumblefish.dev/health` returns 200 (after deploy)
- [x] DNS resolves publicly (records will be in correct hosted zone after deploy)

## Root Cause

Route 53 has two hosted zones:

- `sorobanscan.rumblefish.dev` (Z10396861CRMUIWWA8TL9) — parent
- `staging.sorobanscan.rumblefish.dev` (Z0920117B9QSLT9FXPPW) — child

Parent zone has NS delegation to child zone. CDK created A/AAAA records in the **parent** zone, but DNS queries for `*.staging.sorobanscan.rumblefish.dev` are delegated to the **child** zone where records don't exist → NXDOMAIN.

## Fix

Changed `staging.json`:

- `hostedZoneId`: `Z10396861CRMUIWWA8TL9` → `Z0920117B9QSLT9FXPPW`
- `hostedZoneName`: `sorobanscan.rumblefish.dev` → `staging.sorobanscan.rumblefish.dev`

CDK will now create records in the child zone where DNS actually looks.
