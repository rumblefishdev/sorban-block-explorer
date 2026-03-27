---
url: 'https://orm.drizzle.team/docs/kit-custom-migrations'
title: 'Drizzle Kit — Custom Migrations'
fetched_date: 2026-03-26
task_id: '0007'
---

# Drizzle Kit — Custom Migrations

## Overview

Drizzle Kit enables you to generate empty migration files for writing custom SQL migrations for DDL alterations not yet supported by Drizzle Kit, or for data seeding purposes.

## Generating Custom Migrations

```bash
drizzle-kit generate --custom --name=seed-users
```

Generates a new migration directory with an empty SQL file.

### Example Output Structure

```
drizzle/
  20240309125510_init_sql/
  20240309135510_delicate_seed-users/
```

### Example SQL Migration

```sql
INSERT INTO "users" ("name") VALUES('Dan');
INSERT INTO "users" ("name") VALUES('Andrew');
```

## Running JS/TS Migrations

Support for executing custom JavaScript and TypeScript migration or seeding scripts is planned. Track in GitHub discussion #2832.
