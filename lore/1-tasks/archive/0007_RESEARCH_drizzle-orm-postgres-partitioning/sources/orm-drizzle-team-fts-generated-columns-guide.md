---
url: 'https://orm.drizzle.team/docs/guides/full-text-search-with-generated-columns'
title: 'Drizzle ORM — Full-Text Search with Generated Columns Guide'
fetched_date: 2026-03-26
task_id: '0007'
---

# Full-Text Search with Generated Columns in Drizzle ORM

## Custom Type Definition

```typescript
export const tsvector = customType<{ data: string }>({
  dataType() {
    return `tsvector`;
  },
});
```

## Generated Column Setup

```typescript
bodySearch: tsvector('body_search')
  .notNull()
  .generatedAlwaysAs((): SQL => sql`to_tsvector('english', ${posts.body})`);
```

## GIN Index

```typescript
index('idx_body_search').using('gin', t.bodySearch);
```

## Weighted Columns

```typescript
search: tsvector('search').generatedAlwaysAs(
  (): SQL =>
    sql`setweight(to_tsvector('english', ${posts.title}), 'A')
      || setweight(to_tsvector('english', ${posts.body}), 'B')`
);
```

## Querying

```typescript
const search = 'travel';
await db
  .select()
  .from(posts)
  .where(sql`${posts.search} @@ to_tsquery('english', ${search})`);
```
