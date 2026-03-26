---
url: 'https://docs.aws.amazon.com/AmazonRDS/latest/UserGuide/rds-proxy.howitworks.html'
also_sourced_from: 'https://docs.aws.amazon.com/AmazonRDS/latest/UserGuide/rds-proxy-pinning.html'
title: 'RDS Proxy concepts and terminology'
fetched_date: 2026-03-26
task_id: '0004'
image_count: 0
editorial_note: >
  Pinning details (causes, session_pinning_filters, monitoring metrics)
  are consolidated from the sub-page rds-proxy-pinning.html, not from the
  main concepts page.
---

# RDS Proxy Concepts and Terminology

RDS Proxy handles the network traffic between the client application and the database actively by understanding the database protocol and adjusting behavior based on SQL operations from your application and the result sets from the database.

RDS Proxy reduces memory and CPU overhead for connection management on your database. The database needs fewer resources when applications open many simultaneous connections. It also doesn't require application logic to close and reopen idle connections or to reestablish connections after a database problem.

The RDS Proxy infrastructure is highly available, deployed over multiple Availability Zones (AZs). Its compute resources are serverless, automatically scaling based on your database workload, and are independent of your RDS DB instance.

## Core Concepts

### Connection Pool

The connections that a proxy keeps open and available for your database applications to use make up the **connection pool**.

Each proxy handles connections to a single RDS DB instance and automatically determines the current writer for Multi-AZ configurations.

### Multiplexing (Transaction-Level Reuse)

By default, RDS Proxy can reuse a connection after each transaction in your session. This is called **multiplexing**. When RDS Proxy temporarily removes a connection from the connection pool to reuse it, that operation is called **borrowing** the connection.

### Pinning

In cases where RDS Proxy cannot safely reuse a database connection outside the current session, it keeps the session on the same connection until the session ends. This fallback behavior is called **pinning**. Pinning eliminates the multiplexing benefit.

**Operations that cause pinning in PostgreSQL:**

- `SET` statements (e.g., `SET search_path`, `SET timezone`)
- Prepared statements (PostgreSQL binary protocol)
- `LOCK TABLE` outside transactions
- `CREATE TEMPORARY TABLE`
- Advisory locks (`pg_advisory_lock`)
- Stored procedures returning multiple result sets

**Mitigation:** Use `session_pinning_filters = ["EXCLUDE_VARIABLE_SETS"]` to avoid pinning on benign SET commands from PostgreSQL drivers.

### Proxy Endpoint

A proxy has a default endpoint you connect to instead of connecting to the read/write endpoint that connects directly to the instance. Connect Lambda and other AWS services to this proxy endpoint to take advantage of RDS Proxy functionality.

### Target Group

Each proxy contains a target group. The target group embodies the RDS DB instance the proxy can connect to — these are called the **targets** of the proxy.

### Engine Family

A related set of database engines that use the same DB protocol. Each proxy is associated with one engine family.

## Connection Pooling

Each proxy performs connection pooling separately for the writer and reader instance. Connection pooling reduces overhead associated with:

- Opening and closing connections
- Keeping many connections open simultaneously
- Memory per connection
- CPU overhead from TLS/SSL handshaking, authentication, capability negotiation

Connection pooling also simplifies application logic — you don't need to write code to minimize simultaneous open connections.

### Connection Multiplexing

With multiplexing, RDS Proxy performs all operations for a transaction using one underlying database connection. RDS can then use a different connection for the next transaction. The proxy keeps a **smaller number of connections** open to the DB instance while accepting many simultaneous client connections. This minimizes "too many connections" errors.

**Example:** 500 Lambda clients might use just 20 persistent database connections through a single RDS Proxy.

## Security

RDS Proxy uses existing RDS security mechanisms including TLS/SSL and AWS IAM.

### IAM Authentication Options

**Standard IAM Authentication:**

- Applications authenticate to the proxy using IAM
- Proxy authenticates to the database using credentials from Secrets Manager
- Enforces IAM authentication even if the database uses native password authentication

**End-to-End IAM Authentication:**

- Enforces IAM authentication directly from application to database through the proxy
- Eliminates database credential management in Secrets Manager
- Requires `rds-db:connect` IAM permission on the proxy role

### TLS/SSL

- RDS Proxy uses certificates from AWS Certificate Manager (ACM)
- No need to download Amazon RDS certificates when using RDS Proxy
- Supports TLS 1.0, 1.1, 1.2, and 1.3
- Can connect with higher TLS version than the underlying database supports
- For PostgreSQL: specify `sslmode=require` in connection string

## Failover Handling

RDS Proxy makes applications more resilient to database failovers. When the original DB instance becomes unavailable:

- RDS Proxy connects to the standby without dropping idle application connections
- Continues accepting connections at the same IP address
- Automatically directs connections to the new primary DB instance
- Clients are not susceptible to DNS propagation delays, local DNS caching, or connection timeouts

**Without RDS Proxy:** Failover involves a brief outage, dropped connections, and application-side reconnect logic.

**With RDS Proxy:** Most connections stay alive during failovers. Only connections mid-transaction or mid-statement are cancelled. RDS Proxy queues incoming requests when the writer is unavailable.

## Transaction Semantics

All statements within a single transaction always use the same underlying database connection. The connection becomes available for reuse when the transaction ends.

**Implications:**

- With `autocommit = ON`: Connection reuse can happen after each individual statement
- With `autocommit = OFF`: Connection reuse waits until COMMIT, ROLLBACK, or explicit transaction end
- DDL statements end the transaction implicitly after completion

RDS Proxy detects transaction boundaries through the network protocol, not by parsing SQL keywords.

## Configuration Parameters

| Parameter                      | Purpose                                                | Recommendation                   |
| ------------------------------ | ------------------------------------------------------ | -------------------------------- |
| `max_connections_percent`      | Percentage of RDS `max_connections` available to proxy | 90% (reserve for admin access)   |
| `max_idle_connections_percent` | Idle connection pool threshold                         | 50%                              |
| `connection_borrow_timeout`    | Max wait time for available connection                 | 10–30 seconds (not default 120s) |
| `idle_client_timeout`          | Closes idle client connections                         | 1800 seconds                     |

## PostgreSQL-Specific Limitations

- RDS Proxy does not support session pinning filters for PostgreSQL (except via `EXCLUDE_VARIABLE_SETS`)
- All proxies listen on port **5432** for PostgreSQL
- RDS Proxy does not support cancelling a query from a client via `CancelRequest`
- Prepared statements via the binary protocol cause connection pinning

## Monitoring

Use the CloudWatch metric **`DatabaseConnectionsCurrentlySessionPinned`** to track pinning frequency. High pin ratios indicate that multiplexing efficiency is collapsing toward 1:1, negating the proxy's benefits.
