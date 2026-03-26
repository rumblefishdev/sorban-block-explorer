---
url: 'https://github.com/drizzle-team/drizzle-orm/pull/4355'
title: '[pg-kit] Ignore partition children; keep partition parents (PR #4355)'
fetched_date: 2026-03-26
task_id: '0007'
---

# [pg-kit] Ignore partition children; keep partition parents #4355

**Status:** OPEN
**Part of:** #2854 (partition support feature request)

## Description

Improves `drizzle-kit introspect` (pull) behavior with partitioned tables:

- Copies children's columns to the parent's columns
- Ignores non-local foreign keys (from other tables to partition children), keeping only local FKs (to parent)

## Relevance

Currently `drizzle-kit introspect` sees each partition as a separate table. This PR aims to fix that by recognizing partition parent-child relationships and only exposing the parent table in the generated schema.
