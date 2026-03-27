---
url: 'https://orm.drizzle.team/docs/column-types/pg'
title: 'Drizzle ORM — PostgreSQL Column Types (JSONB)'
fetched_date: 2026-03-26
task_id: '0007'
---

# Drizzle ORM — PostgreSQL JSONB Column Type

## Basic Usage

```typescript
import { jsonb, pgTable } from 'drizzle-orm/pg-core';

const table = pgTable('table', {
  jsonb1: jsonb(),
  jsonb2: jsonb().default({ foo: 'bar' }),
  jsonb3: jsonb().default(sql`'{foo: "bar"}'::jsonb`),
});
```

Generates:

```sql
CREATE TABLE "table" (
  "jsonb1" jsonb,
  "jsonb2" jsonb default '{"foo": "bar"}'::jsonb,
  "jsonb3" jsonb default '{"foo": "bar"}'::jsonb
);
```

## TypeScript Type Safety with `.$type<T>()`

```typescript
// Typed as { foo: string }
jsonb: jsonb().$type<{ foo: string }>();

// Typed as string[]
jsonb: jsonb().$type<string[]>();

// Won't compile - type mismatch
jsonb: jsonb().$type<string[]>().default({});
```

Provides compile-time protection for default values, insert and select schemas. No runtime validation — pair with Zod for that.
