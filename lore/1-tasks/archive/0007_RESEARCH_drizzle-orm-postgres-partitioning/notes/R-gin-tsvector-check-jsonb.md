---
title: 'Drizzle ORM: GIN Indexes, tsvector, CHECK, JSONB'
type: research
status: mature
spawned_from: '0007'
spawns: []
tags: [drizzle, gin, tsvector, jsonb, check-constraints]
links: []
history: []
---

# Drizzle ORM: GIN Indexes, tsvector, CHECK, JSONB

## GIN Indexes: YES (native)

Fully supported via `index().using('gin', ...)`. Works for JSONB, tsvector, arrays.

```typescript
import { index, pgTable, jsonb } from 'drizzle-orm/pg-core';

export const events = pgTable(
  'soroban_events',
  {
    topics: jsonb('topics'),
  },
  (t) => [
    index('idx_topics_gin').using('gin', t.topics),
    // With operator class:
    // Note: 'name' column not defined in this example — in practice, target an existing text column
    index('idx_name_trgm').using('gin', t.name.op('gin_trgm_ops')),
  ]
);
```

**Caveat:** Operator classes (like `gin_trgm_ops`) require drizzle-kit >= 0.28.0 (fixed in [#2935](https://github.com/drizzle-team/drizzle-orm/issues/2935)).

## tsvector Generated Columns: YES (native, since v0.32.0)

Drizzle supports `GENERATED ALWAYS AS ... STORED` via `.generatedAlwaysAs()`. No built-in `tsvector` type — use `customType`.

**API stability note:** In v1.0.0-beta.12+, `generatedAlwaysAs()` no longer accepts raw string literals — only `sql` tagged templates and callbacks. The callback form shown below is the forward-compatible pattern.

```typescript
import { sql, SQL } from 'drizzle-orm';
import { customType, index, jsonb, pgTable, text } from 'drizzle-orm/pg-core';

const tsvector = customType<{ data: string }>({
  dataType() {
    return 'tsvector';
  },
});

export const contracts = pgTable(
  'soroban_contracts',
  {
    metadata: jsonb('metadata').$type<{ name?: string }>(),
    searchVector: tsvector('search_vector').generatedAlwaysAs(
      (): SQL =>
        sql`to_tsvector('english', coalesce(${contracts.metadata}->>'name', ''))`
    ),
  },
  (t) => [index('idx_search_vector').using('gin', t.searchVector)]
);
```

**Key points:**

- Use callback form `(): SQL => sql\`...\`` when referencing columns in same table
- JSONB field extraction (`metadata->>'name'`) works inside `sql` template — raw SQL is passed through
- The `coalesce()` handles null metadata gracefully

## CHECK Constraints: PARTIAL

`check()` exists in schema API but drizzle-kit has reliability issues emitting CHECK in migrations.

```typescript
import { sql } from 'drizzle-orm';
import { check, pgTable, varchar } from 'drizzle-orm/pg-core';

export const tokens = pgTable(
  'tokens',
  {
    assetType: varchar('asset_type', { length: 10 }).notNull(),
  },
  (t) => [
    check(
      'asset_type_check',
      sql`${t.assetType} IN ('classic', 'sac', 'soroban')`
    ),
  ]
);
```

**Caveats:**

- **MUST use array syntax** `(t) => [check(...)]` — the old object syntax `(t) => ({...})` silently drops CHECK constraints from migrations. This is the #1 pitfall (still tripping users as of June 2025 per [#3520](https://github.com/drizzle-team/drizzle-orm/issues/3520) comments).
- Even with array syntax, drizzle-kit may occasionally skip CHECK in generated migrations
- **Workaround:** Always verify generated SQL contains the CHECK. If missing, add manually to migration file.

## JSONB Column Typing: YES (native)

Built-in `jsonb()` with TypeScript generics via `.$type<T>()`.

```typescript
interface ContractMetadata {
  name?: string;
  version?: string;
  functions: Array<{ name: string; inputs: string[] }>;
}

export const contracts = pgTable('soroban_contracts', {
  metadata: jsonb('metadata').$type<ContractMetadata>().notNull(),
  // Array type
  balances: jsonb('balances').$type<Record<string, string>>().default({}),
});
```

**Caveats:**

- `.$type<T>()` is compile-time only — no runtime validation
- Querying nested JSONB fields requires raw `sql` and loses type safety
- Pair with Zod for runtime validation at system boundaries

**Schema discipline note:** The compile-time typing via `.$type<T>()` supports the schema evolution rule "avoid replacing explicit relational structure with oversized generic JSON blobs" — TypeScript will flag type mismatches when JSONB structures drift. However, runtime validation (Zod) is needed at ingestion boundaries to enforce this at data level.

## Sources

- [Drizzle Generated Columns](https://orm.drizzle.team/docs/generated-columns)
- [Drizzle Full-text Search Guide](https://orm.drizzle.team/docs/guides/full-text-search-with-generated-columns)
- [Drizzle Indexes & Constraints](https://orm.drizzle.team/docs/indexes-constraints)
- [Drizzle Column Types - jsonb](https://orm.drizzle.team/docs/column-types/pg)
