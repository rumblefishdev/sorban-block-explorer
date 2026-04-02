# Database Migrations

sqlx-cli manages plain SQL migrations in `crates/db/migrations/`.

## Prerequisites

```bash
cargo install sqlx-cli --no-default-features --features postgres
docker compose up -d   # local PostgreSQL
```

## Commands (from repo root)

| Command | Description |
|---------|-------------|
| `npm run db:migrate` | Apply pending migrations |
| `npm run db:revert` | Revert the most recent migration |
| `npm run db:status` | Show applied/pending migrations |
| `npm run db:add -- <name>` | Create a new reversible migration |
| `npm run db:prepare` | Regenerate `.sqlx/` for offline CI builds |
| `npm run db:reset` | Drop DB, recreate, apply all migrations |

## Creating a new migration

```bash
npm run db:add -- create_users
```

This creates a pair of files:

```
crates/db/migrations/
  YYYYMMDDHHMMSS_create_users.up.sql
  YYYYMMDDHHMMSS_create_users.down.sql
```

Write the schema change in `.up.sql` and its reverse in `.down.sql`.

## Conventions

- Migrations 0001-0004 are irreversible (initial schema, never revert).
- All new migrations use the `-r` flag (reversible, paired up/down files).
- sqlx-cli generates timestamp prefixes. These sort after the numeric 0001-0004 prefixes.
- After adding or changing `sqlx::query!()` calls, regenerate offline data: `npm run db:prepare`

## Offline builds (SQLX_OFFLINE)

The `.sqlx/` directory at the repo root contains type metadata for compile-time checked queries. CI builds use `SQLX_OFFLINE=true` to compile without a live database.

After modifying queries or migrations:

```bash
npm run db:migrate    # apply new migrations
npm run db:prepare    # regenerate .sqlx/
git add .sqlx/        # commit updated offline data
```

## Production rollback

Migrations run automatically via CDK custom resource before Lambda deployments. If a migration fails, the deployment is aborted (old code continues running).

To roll back a migration in staging/production:
1. Connect to the database through a bastion host or VPN
2. Run `sqlx migrate revert --source crates/db/migrations`
3. Deploy the previous code version
