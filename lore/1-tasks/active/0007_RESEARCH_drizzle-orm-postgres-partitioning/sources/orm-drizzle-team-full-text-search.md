---
url: 'https://orm.drizzle.team/docs/guides/postgresql-full-text-search'
title: 'Drizzle ORM — PostgreSQL Full-Text Search Guide'
fetched_date: 2026-03-26
task_id: '0007'
---

# PostgreSQL Full-Text Search with Drizzle ORM

## Core Concepts

- `to_tsvector`: parses text into tokens/lexemes, returns tsvector
- `to_tsquery`: converts keywords into normalized tokens, returns tsquery
- `@@` operator: matches tsvector against tsquery

## GIN Index for Performance

```typescript
import { index, pgTable, serial, text } from 'drizzle-orm/pg-core';

export const posts = pgTable(
  'posts',
  {
    id: serial('id').primaryKey(),
    title: text('title').notNull(),
  },
  (table) => [
    index('title_search_index').using(
      'gin',
      sql`to_tsvector('english', ${table.title})`
    ),
  ]
);
```

## Multi-column Search with Weights

```typescript
index('search_index').using(
  'gin',
  sql`(
  setweight(to_tsvector('english', ${table.title}), 'A') ||
  setweight(to_tsvector('english', ${table.description}), 'B')
)`
);
```

## Query Functions

- `to_tsquery`: simple keyword matching
- `plainto_tsquery`: multi-keyword AND
- `phraseto_tsquery`: exact phrase match
- `websearch_to_tsquery`: web-style syntax with OR support

## Ranking

- `ts_rank`: term frequency
- `ts_rank_cd`: term proximity

## Requirements

PostgreSQL 9.6+ / 10.0+ for full feature support.
