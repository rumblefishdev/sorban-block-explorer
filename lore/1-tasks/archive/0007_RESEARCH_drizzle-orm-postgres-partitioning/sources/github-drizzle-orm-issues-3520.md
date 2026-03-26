---
url: 'https://github.com/drizzle-team/drizzle-orm/issues/3520'
title: '[BUG]: Check constraints not being generated for postgres'
fetched_date: 2026-03-26
task_id: '0007'
---

# [BUG]: Check constraints not being generated for postgres #3520

**Status:** CLOSED
**Versions:** drizzle-orm 0.36.1, drizzle-kit 0.28.0
**Comments:** 25

## Description

Check constraints are not being generated when using drizzle with PostgreSQL. Users report that `drizzle-kit generate` does not emit CHECK constraints into migration files.

## Root Cause

The issue was related to using the **object syntax** instead of the **array syntax** for table constraints. The array syntax `(t) => [check(...)]` is required for drizzle-kit to pick up constraints.

## Workaround

1. Use array syntax: `(t) => [check('name', sql`...`)]`
2. If still not generated, add CHECK manually to the migration SQL file
3. Or create the constraint directly in PostgreSQL and run `drizzle-kit pull`
