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

- Migrations 0001-0007 are irreversible (initial schema per ADR 0027, never revert).
- All new migrations use the `-r` flag (reversible, paired up/down files).
- sqlx-cli generates timestamp prefixes. These sort after the numeric 0001-0007 prefixes.
- After adding or changing `sqlx::query!()` calls, regenerate offline data: `npm run db:prepare`

## Initial schema (ADR 0027)

The 0001-0007 migrations produce the schema defined by ADR 0027 (post-surrogate snapshot). Pre-ADR migrations (the previous 0001-0009 chain) were wiped under task 0140 and archived locally to `.trash/migrations-pre-adr-0027/`. `.trash/` is git-ignored (per project deletion policy), so the authoritative record of the old chain is git history itself — inspect any commit before `89f4335` to recover the pre-ADR migrations.

| #    | File                                   | Tables                                                                                             |
| ---- | -------------------------------------- | -------------------------------------------------------------------------------------------------- |
| 0001 | `0001_extensions.sql`                  | `pg_trgm` extension                                                                                |
| 0002 | `0002_identity_and_ledgers.sql`        | `ledgers`, `accounts`, `wasm_interface_metadata`, `soroban_contracts`                              |
| 0003 | `0003_transactions_and_operations.sql` | `transactions`, `transaction_hash_index`, `operations_appearances` (ADR 0163), `transaction_participants` |
| 0004 | `0004_soroban_activity.sql`            | `soroban_events_appearances` (ADR 0033), `soroban_invocations_appearances` (ADR 0034)              |
| 0005 | `0005_tokens_nfts.sql`                 | `assets` (ADR 0036; renamed from `tokens`), `nfts`, `nft_ownership`                                |
| 0006 | `0006_liquidity_pools.sql`             | `liquidity_pools`, `liquidity_pool_snapshots`, `lp_positions` (+ deferred `operations_appearances.pool_id` FK) |
| 0007 | `0007_account_balances.sql`            | `account_balances_current` (ADR 0035 dropped `account_balance_history`)                            |

Partitioned tables (`transactions`, `operations_appearances`, `transaction_participants`, `soroban_events_appearances`, `soroban_invocations_appearances`, `nft_ownership`, `liquidity_pool_snapshots`) create the parent only. Monthly partitions are provisioned by the partition-management Lambda (`crates/db-partition-mgmt`).

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
