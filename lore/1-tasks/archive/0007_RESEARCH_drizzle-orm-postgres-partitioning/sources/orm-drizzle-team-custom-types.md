---
url: 'https://orm.drizzle.team/docs/custom-types'
title: 'Drizzle ORM — Custom Types'
fetched_date: 2026-03-26
task_id: '0007'
---

# Drizzle ORM — Custom Types

Used to define column types not natively supported by Drizzle (e.g., `tsvector`).

## API

```typescript
import { customType } from 'drizzle-orm/pg-core';

const tsvector = customType<{ data: string }>({
  dataType() {
    return 'tsvector';
  },
});
```

The generic parameter `{ data: string }` defines the TypeScript type for the column value. `dataType()` returns the SQL type name.

## Relevance

Required for `tsvector` columns since Drizzle has no built-in `tsvector` type. Works with `generatedAlwaysAs()`, GIN indexes, and all standard column operations.
