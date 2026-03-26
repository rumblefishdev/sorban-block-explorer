---
url: 'https://orm.drizzle.team/docs/indexes-constraints'
title: 'Drizzle ORM — Indexes & Constraints (PostgreSQL)'
fetched_date: 2026-03-26
task_id: '0007'
---

# Drizzle ORM — Indexes & Constraints

## CHECK Constraints

```typescript
import { sql } from 'drizzle-orm';
import { check, integer, pgTable, text, uuid } from 'drizzle-orm/pg-core';

export const users = pgTable(
  'users',
  {
    id: uuid().defaultRandom().primaryKey(),
    username: text().notNull(),
    age: integer(),
  },
  (table) => [check('age_check1', sql`${table.age} > 21`)]
);
```

Generates:

```sql
CREATE TABLE "users" (
  "id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
  "username" text NOT NULL,
  "age" integer,
  CONSTRAINT "age_check1" CHECK ("users"."age" > 21)
);
```

## Unique Constraints

```typescript
export const user = pgTable('user', {
  id: integer('id').unique(),
});

// Composite unique
export const composite = pgTable(
  'composite_example',
  {
    id: integer('id'),
    name: text('name'),
  },
  (t) => [unique().on(t.id, t.name)]
);
```

PostgreSQL 15.0+ supports `NULLS NOT DISTINCT`.

## Indexes

```typescript
export const user = pgTable(
  'user',
  {
    id: serial('id').primaryKey(),
    name: text('name'),
    email: text('email'),
  },
  (table) => [
    index('name_idx').on(table.name),
    uniqueIndex('email_idx').on(table.email),
  ]
);
```

GIN index with operator class:

```typescript
index('name_trgm').using('gin', table.name.op('gin_trgm_ops'));
```

All index fields supported since drizzle-kit 0.29.0.
