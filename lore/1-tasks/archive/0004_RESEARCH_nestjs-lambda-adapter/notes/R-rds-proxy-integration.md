---
prefix: R
title: 'RDS Proxy Integration with Lambda'
status: mature
spawned_from: '0004'
---

# R: RDS Proxy Integration with Lambda

## How RDS Proxy Works

RDS Proxy sits between Lambda and RDS PostgreSQL, providing:

1. **Connection multiplexing** — Reuses database connections across Lambda instances at transaction boundaries. 500 Lambda clients might share just 20 real DB connections.
2. **Connection pooling** — Reduces overhead of opening/closing connections (TLS handshake, authentication, capability negotiation).
3. **Failover handling** — Connects to standby without dropping idle connections during failover.

> Source: [rds-proxy-concepts-aws-docs.md](../sources/rds-proxy-concepts-aws-docs.md), "Core Concepts" and "Connection Pooling" sections

## Connection Pinning: The Key Risk

Pinning locks a session to one database connection, preventing multiplexing. **Operations that cause pinning in PostgreSQL:**

- `SET` statements (e.g., `SET search_path`, `SET timezone`)
- **Prepared statements (PostgreSQL binary protocol)** — this is why `postgres.js` requires `prepare: false`
- `LOCK TABLE` outside transactions
- `CREATE TEMPORARY TABLE`
- Advisory locks (`pg_advisory_lock`)

> Source: [rds-proxy-concepts-aws-docs.md](../sources/rds-proxy-concepts-aws-docs.md), "Pinning" section

### Mitigation for SET Statements

Use `session_pinning_filters = ["EXCLUDE_VARIABLE_SETS"]` on the proxy to avoid pinning on benign SET commands from PostgreSQL drivers.

> Source: [rds-proxy-concepts-aws-docs.md](../sources/rds-proxy-concepts-aws-docs.md), "Pinning" section — "Mitigation"

### Monitoring Pinning

Monitor CloudWatch metric `DatabaseConnectionsCurrentlySessionPinned`. High values indicate pinning is degrading proxy efficiency.

> Source: [rds-proxy-concepts-aws-docs.md](../sources/rds-proxy-concepts-aws-docs.md), line 142 — `DatabaseConnectionsCurrentlySessionPinned` metric

## Application-Side Configuration for RDS Proxy

| Setting                      | Value                           | Reason                                                    |
| ---------------------------- | ------------------------------- | --------------------------------------------------------- |
| `host`                       | RDS Proxy endpoint              | Not the RDS instance directly                             |
| `max` (pool size)            | 1                               | Lambda is single-concurrent; proxy does real pooling      |
| `ssl`                        | `{ rejectUnauthorized: false }` | RDS Proxy uses ACM certificates, different from RDS certs |
| `prepare` (postgres.js only) | `false`                         | Prevents binary prepared statement pinning                |
| Driver                       | `node-postgres` (pg)            | No binary prepared statements by default                  |

> Sources: [node-postgres-lambda-rds-proxy-best-practices.md](../sources/node-postgres-lambda-rds-proxy-best-practices.md), "Pool Sizing for Lambda" section; [rds-proxy-concepts-aws-docs.md](../sources/rds-proxy-concepts-aws-docs.md), "Pinning" section (prepared statements as pinning cause)

## Node.js 20+ SSL Certificate Requirement

Node.js 20+ Lambda runtime requires explicit CA certificate configuration for RDS:

```
NODE_EXTRA_CA_CERTS=/var/runtime/ca-cert.pem
```

This environment variable must be set in the Lambda function configuration.

> Source: [aws-lambda-rds-official-docs.md](../sources/aws-lambda-rds-official-docs.md) — Node.js 20+ CA certificate handling

**Note:** When connecting through RDS Proxy (not directly to RDS), the proxy uses ACM certificates. The `ssl: { rejectUnauthorized: false }` pattern bypasses strict certificate validation, which is acceptable for internal VPC traffic between Lambda and RDS Proxy.

## Authentication Options

Two IAM auth modes:

1. **Standard** — Lambda authenticates to proxy via IAM; proxy authenticates to DB via Secrets Manager credentials
2. **End-to-End IAM** — IAM authentication from Lambda all the way to the database; eliminates Secrets Manager dependency

> Source: [rds-proxy-concepts-aws-docs.md](../sources/rds-proxy-concepts-aws-docs.md), "IAM Authentication Options" section

For our project, standard IAM auth with Secrets Manager is simpler and sufficient for a read-only API.

**RDS Proxy is the clear choice** for Lambda + RDS PostgreSQL — it is the only managed, Lambda-native connection proxy for PostgreSQL with built-in IAM auth and automatic failover.

> Source: [rds-proxy-concepts-aws-docs.md](../sources/rds-proxy-concepts-aws-docs.md), "Core Concepts" — managed, multi-AZ, serverless scaling
