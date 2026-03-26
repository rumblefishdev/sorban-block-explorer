---
url: 'https://orm.drizzle.team/docs/migrations'
title: 'Drizzle ORM — Migrations'
fetched_date: 2026-03-26
task_id: '0007'
---

# Drizzle ORM — Migrations

## Overview

SQL databases require a strict schema. Drizzle supports multiple migration approaches: database-first and codebase-first.

## Drizzle Kit Commands

- `drizzle-kit generate` — create SQL migration files from TS schema diff
- `drizzle-kit migrate` — apply pending migrations
- `drizzle-kit push` — apply schema directly (no migration files, dev only)
- `drizzle-kit pull` — generate TS schema from existing database

## Migration Options

### Option 1: Pull Schema (database-first)

Use `drizzle-kit pull` to generate TypeScript schema from database.

### Option 2: Push (rapid prototyping)

`drizzle-kit push` applies changes without SQL files. Not for production.

### Option 3: Generate & Apply (recommended for production)

`drizzle-kit generate` creates SQL files, `drizzle-kit migrate` applies them.

### Option 4: Runtime Migrations

```typescript
import { migrate } from 'drizzle-orm/node-postgres/migrator';
await migrate(db);
```

### Option 5: External Tool Integration

Generate SQL with `drizzle-kit generate`, apply with Bytebase, Liquibase, or Atlas.
