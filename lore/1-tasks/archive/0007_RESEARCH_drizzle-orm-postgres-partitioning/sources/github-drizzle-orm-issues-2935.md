---
url: 'https://github.com/drizzle-team/drizzle-orm/issues/2935'
title: '[BUG]: drizzle-kit generate ignores index operators'
fetched_date: 2026-03-26
task_id: '0007'
---

# [BUG]: drizzle-kit generate ignores index operators #2935

**Status:** CLOSED
**Versions:** drizzle-orm 0.33.0, drizzle-kit 0.24.2
**Comments:** 4

## Description

Creating an index with an operator class (e.g., `gin_trgm_ops`) results in the wrong migration output. The operator class is silently dropped.

```typescript
index().using('gin', table.name.op('gin_trgm_ops'));
```

Generated migration was missing the operator class entirely.

## Resolution

Fixed in drizzle-kit 0.28.0. Operator classes in indexes are now correctly emitted in generated migrations.
