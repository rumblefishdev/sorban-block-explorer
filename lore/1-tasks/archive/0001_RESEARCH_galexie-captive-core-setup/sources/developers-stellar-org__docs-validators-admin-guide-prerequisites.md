---
url: 'https://developers.stellar.org/docs/validators/admin-guide/prerequisites'
title: 'Prerequisites | Stellar Docs'
fetched_date: 2026-03-25
task_id: '0001'
image_count: 0
---

# Prerequisites | Stellar Docs

## Overview

You can install Stellar Core in [multiple ways](https://developers.stellar.org/docs/validators/admin-guide/installation), and once installed, you can [configure](https://developers.stellar.org/docs/validators/admin-guide/configuring) it to participate at [different levels](https://developers.stellar.org/docs/validators#types-of-nodes): either as a Basic Validator or Full Validator. Regardless of installation method or node type, you must "set up and connect to the peer-to-peer network and store the state of the ledger in a SQL [database](https://developers.stellar.org/docs/validators/admin-guide/configuring#database)."

## Hardware Requirements

> "CPU, RAM, Disk and network depends on network activity. If you decide to collocate certain workloads, you will need to take this into account."

As of April 2024, a [c5d.2xlarge](https://aws.amazon.com/ec2/instance-types/c5/) AWS instance (8x Intel Xeon vCPUs at 3.4 GHz; 16 GB RAM; 100 GB NVMe SSD with 10,000 iops) worked well running Stellar Core with PostgreSQL on the same machine.

Stellar Core runs on modest hardware to enable broad participation. Basic nodes function without significant overhead, though greater demands require increased resources.

| Node Type           | CPU                     | RAM   | Disk            | AWS SKU                                                      | Google Cloud SKU                                                                          |
| ------------------- | ----------------------- | ----- | --------------- | ------------------------------------------------------------ | ----------------------------------------------------------------------------------------- |
| Core Validator Node | 8x Intel Xeon @ 3.4 GHz | 16 GB | 100 GB NVMe SSD | [c5d.2xlarge](https://aws.amazon.com/ec2/instance-types/c5/) | [n4-highcpu-8](https://cloud.google.com/compute/docs/general-purpose-machines#n4-highcpu) |

_\* Assuming a 30-day retention window for data storage._

## Stellar Network Access

Stellar Core syncs a distributed ledger via peer-to-peer network interaction, requiring "your node needs to make certain [TCP ports](https://en.wikipedia.org/wiki/Transmission%5FControl%5FProtocol#TCP%5Fports) available for inbound and outbound communication."

### Inbound

A Stellar Core node must allow all IP addresses to connect to its `PEER_PORT` over TCP. While configurable during [setup](https://developers.stellar.org/docs/validators/admin-guide/configuring), the default is **11625**.

### Outbound

A Stellar Core node must connect to other nodes via their `PEER_PORT` over TCP. Network explorers like [Stellarbeat](https://stellarbeat.io/) provide peer information; most nodes use the default port **11625**.

## Internal System Access

Stellar Core connects to internal systems; implementation varies based on your configuration.

### Inbound

- Stellar Core exposes an _unauthenticated_ HTTP endpoint on its `HTTP_PORT`. The default is **11626**, configurable during [setup](https://developers.stellar.org/docs/validators/admin-guide/configuring).
- The `HTTP_PORT` allows systems like Stellar RPC to submit transactions, potentially requiring exposure to internal IP addresses.
- It supports querying Stellar Core [info](https://developers.stellar.org/docs/validators/admin-guide/commands) and obtaining [metrics](https://developers.stellar.org/docs/validators/admin-guide/monitoring).
- Administrative commands such as [scheduling upgrades](https://developers.stellar.org/docs/validators/admin-guide/network-upgrades) and adjusting log levels use this port.

> "If you need to expose this endpoint to other hosts in your local network, we strongly recommended you use an intermediate reverse proxy server to implement authentication. Don't expose the HTTP endpoint to the raw and cruel open internet."

### Outbound

- Stellar Core requires database access (PostgreSQL recommended). If your database resides on a different network machine, permit that connection, specified during [configuration](https://developers.stellar.org/docs/validators/admin-guide/configuring).
- Block all other outbound connections safely.

## Storage

Most storage needs stem from stellar-core's database backend, which stores complete ledger state. Both database and related directories (like `buckets`) are entirely managed by Stellar Core and may be disregarded. As of April 2024, 100 GB suffices.

### Database

The database is consulted during consensus and atomically modified when transaction sets apply to the ledger. It provides random access, fine-grained operations, and speed.

**As of August 2024, Stellar Core officially transitioned to BucketListDB as its primary database backend.** BucketListDB still requires a SQL database—either SQLite or Postgres (recommended).

For PostgreSQL, configure local database access via Unix domain socket and update these parameters:

```
# !!! DB connection should be over a Unix domain socket !!!
# shared_buffers = 25% of available system ram
# effective_cache_size = 50% of available system ram
# max_wal_size = 5GB
# max_connections = 150
```

### Buckets

Stellar-core stores ledger state in flat XDR files called "buckets." These files support hashing and ledger difference transmission to history archives. With BucketListDB, the `buckets` directory serves as a primary database backend. With SQL, the `buckets` directory exists only for hashing and history archives, representing a ledger state copy stored in SQL.

Store buckets on fast, local disk with space for several times the current ledger size. Current ledger size is approximately 10 GB (April 2024); plan accordingly.

## Kubernetes Considerations

Currently, validator node operation in Kubernetes is not recommended. Should you proceed, consider:

- Sensitive data like node seeds store in Kubernetes etcd. Use credential tools such as [vault agent](https://developer.hashicorp.com/vault/docs/platform/k8s/injector) or [AWS Secrets Store CSI driver](https://github.com/aws/secrets-store-csi-driver-provider-aws) to enhance security
- Plan how external traffic reaches pods. Tier 1 nodes need public DNS names with internet-accessible ports
- Validators have unique seeds and history archive configurations, requiring individual pod configurations
- Ensure sufficient resources constantly available to pods
- Depending on history archive publishing, you may need custom docker images with additional tooling
