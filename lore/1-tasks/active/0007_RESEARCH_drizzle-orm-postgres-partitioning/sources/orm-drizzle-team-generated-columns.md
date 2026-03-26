---
url: 'https://orm.drizzle.team/docs/generated-columns'
title: 'Drizzle ORM — Generated Columns'
fetched_date: 2026-03-26
task_id: '0007'
---

# Drizzle ORM — Generated Columns

## Overview

Generated columns compute values automatically. Two types:

- **Virtual**: computed during queries (no storage)
- **Stored**: computed on insert/update, persisted

## Database Support

| Database   | Types           | Notes                                    |
| ---------- | --------------- | ---------------------------------------- |
| PostgreSQL | STORED only     | Cannot modify expressions after creation |
| MySQL      | STORED, VIRTUAL | Both indexable                           |
| SQLite     | STORED, VIRTUAL | Cannot change STORED expressions         |

## API: `.generatedAlwaysAs()`

### Three Expression Formats

**String literal:**

```typescript
text('gen_name').generatedAlwaysAs(`'hello world!'`);
```

**SQL template (for escaping):**

```typescript
text('gen_name').generatedAlwaysAs(sql`'hello "world"!'`);
```

**Callback with column references:**

```typescript
text('gen_name').generatedAlwaysAs(
  (): SQL => sql`'hi, ' || ${test.name} || '!'`
);
```

## Full-Text Search Example (PostgreSQL)

```typescript
const tsVector = customType<{ data: string }>({
  dataType() {
    return 'tsvector';
  },
});

export const test = pgTable(
  'test',
  {
    id: integer('id').primaryKey().generatedAlwaysAsIdentity(),
    content: text('content'),
    contentSearch: tsVector('content_search').generatedAlwaysAs(
      (): SQL => sql`to_tsvector('english', ${test.content})`
    ),
  },
  (t) => [index('idx_content_search').using('gin', t.contentSearch)]
);
```

## Limitations

- PostgreSQL: Cannot reference other generated columns or use subqueries
- Cannot use in primary/foreign keys or unique constraints
- `drizzle-kit push`: Cannot change expressions/types (requires drop-recreate)
- `drizzle-kit generate`: No limitations
