---
id: '0080'
title: 'Fix GitHub Pages deploy failing with duplicate artifacts'
type: BUG
status: completed
related_adr: []
related_tasks: ['0079']
tags: [priority-high, effort-small, layer-tooling]
links: []
history:
  - date: 2026-03-25
    status: active
    who: stkrolikiewicz
    note: 'Deploy fails with: Multiple artifacts named github-pages were unexpectedly found'
  - date: 2026-03-25
    status: completed
    who: stkrolikiewicz
    note: 'Split into build and deploy jobs, deployed successfully'
---

# Fix GitHub Pages deploy failing with duplicate artifacts

## Summary

The `deploy-board.yml` workflow fails after merge to develop with error: "Multiple artifacts named github-pages were unexpectedly found for this workflow run."

## Root Cause

`upload-pages-artifact` and `deploy-pages` run in the same job. When concurrent pushes or race conditions occur, multiple artifacts with the same name are created.

## Fix

Split into separate `build` and `deploy` jobs. The deploy job depends on build, ensuring a single artifact upload per workflow run.

## Acceptance Criteria

- [x] `deploy-board.yml` splits upload and deploy into separate jobs
- [x] Board deploys successfully to GitHub Pages after merge to develop
