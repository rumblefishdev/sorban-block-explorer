---
prefix: R
title: 'ECS Fargate task sizing for Galexie with Captive Core'
status: developing
spawned_from: null
spawns: []
---

# ECS Fargate Task Sizing for Galexie with Captive Core

## Summary

Galexie runs Captive Core (stellar-core in embedded/captive mode) as a subprocess to stream ledger data and write it to S3. Captive Core is memory-intensive and needs fast local disk for bucket files. This note covers hardware sizing, Fargate constraints, VPC networking, and recommended task definition settings.

---

## 1. Compute Requirements

### Official Galexie Minimum Hardware (from Stellar docs)

Source: [Galexie Prerequisites | Stellar Docs](https://developers.stellar.org/docs/data/indexers/build-your-own/galexie/admin_guide/prerequisites)

| Resource        | Minimum                              |
| --------------- | ------------------------------------ |
| RAM             | **16 GB**                            |
| CPU             | **4 vCPUs**                          |
| Persistent Disk | **100 GB** with at least **5K IOPS** |

> Note: An earlier version of the docs listed 8 GB / 2 vCPU. The current docs (March 2026) list **16 GB / 4 vCPU** as the minimum. Always verify against live docs.

### Full Stellar Core Validator Baseline (for comparison)

> **Source**: Confirmed in `developers-stellar-org__docs-validators-admin-guide-prerequisites.md`.

AWS reference instance: **c5d.2xlarge** (8 vCPU, 16 GB RAM, 100 GB NVMe SSD at 10,000 IOPS). This is for a full validator node, not Captive Core. Included for context only.

### Why Captive Core Needs Both RAM and Disk

Captive Core requires **significant RAM and disk simultaneously**:

- **RAM**: The active ledger state is loaded into memory for processing. The `pkgdev-ledgerbackend.md` source estimates ~3 GB for BoundedRange (as of August 2020); the current mainnet state is likely larger but no recent figure is confirmed in our sources.
- **Disk**: Captive Core stores bucket files on local disk. The official prerequisites specify 100 GB at 5K IOPS without breaking down the RAM vs disk allocation.

> **Source**: BucketListDB officially became primary backend as of August 2024 (confirmed by `developers-stellar-org__docs-validators-admin-guide-prerequisites.md`). The 16 GB RAM + 100 GB disk requirement is confirmed by `stellar-docs-galexie-prerequisites.md`.

### Fargate Task Size Recommendation

Based on the 16 GB / 4 vCPU minimum, the nearest valid Fargate combination is:

| Setting | Value                 |
| ------- | --------------------- |
| CPU     | **4096** (4 vCPU)     |
| Memory  | **16384 MiB** (16 GB) |

Valid memory range for 4 vCPU Fargate tasks: **8 GB to 30 GB** in 1 GB increments.

For production headroom, **20 GB** (20480 MiB) with 4 vCPU is a safer starting point if budget allows.

---

## 2. Disk / Storage Requirements

### What Captive Core Needs on Disk

Captive Core stores ledger bucket files on local disk. These are flat XDR files used for hashing and history archives. Key facts:

- Official guidance: **100 GB total disk at ≥5K IOPS** (source: `stellar-docs-galexie-prerequisites.md`)
- The `pkgdev-ledgerbackend.md` source mentions a `StoragePath` Go config field that maps to Core's `BUCKET_DIR_PATH` with `/captive-core` appended. In the Galexie Docker image, this is set via the `CAPTIVE_CORE_STORAGE_PATH` environment variable in the task definition.

### Fargate Ephemeral Storage

Source: [Fargate task ephemeral storage | AWS docs](https://docs.aws.amazon.com/AmazonECS/latest/developerguide/fargate-task-storage.html)

| Platform version | Default ephemeral storage | Max configurable |
| ---------------- | ------------------------- | ---------------- |
| 1.4.0 or later   | 20 GiB                    | **200 GiB**      |
| 1.3.0 or earlier | 14 GB (10 + 4)            | Not configurable |

Important constraints:

- The container image size counts against the ephemeral storage total.
- Ephemeral storage is **not persistent** — it is lost when the task stops. This is acceptable for Captive Core bucket files because they can be re-fetched from history archives, but cold-start times will be longer.
- Storage is encrypted with AES-256 (AWS-owned key or CMK) for platform version 1.4.0+ tasks launched on May 28, 2020 or later.
- The default minimum is 20 GiB (source: `aws-docs-fargate-ephemeral-storage.md`).

### Recommended Ephemeral Storage Setting

```json
"ephemeralStorage": {
  "sizeInGiB": 100
}
```

This matches the 100 GB official minimum and leaves room for the container image, captive core bucket files, and Galexie working files. Platform version must be `LATEST` (≥1.4.0).

### IOPS Caveat

The Galexie prerequisites specify **≥5K IOPS**. Fargate ephemeral storage is SSD-backed but **AWS does not publish IOPS guarantees** for Fargate ephemeral storage. There is no way to configure provisioned IOPS on Fargate (unlike EBS on EC2). In practice, SSD-backed ephemeral storage on Fargate is likely sufficient for Captive Core's bucket file I/O, but this is an unverified assumption. If I/O becomes a bottleneck (observable via slow `galexie_ingest_ledger_fetch_duration_seconds` metrics), the fallback would be migrating to EC2 with provisioned IOPS EBS, or using EFS in Max I/O mode.

### No EFS Needed for Stateless Galexie

Galexie is designed to be stateless and restartable. If a task terminates mid-export, it can resume without corruption. Persistent EFS volumes are not required unless you want to persist bucket files across task restarts to speed up cold starts. For a simpler setup, ephemeral storage is sufficient.

---

## 3. VPC Networking Requirements

### Overview

Galexie's networking requirements differ between live and backfill modes:

| Mode                                   | Path                  | Protocol                                                               | Why                                                                        |
| -------------------------------------- | --------------------- | ---------------------------------------------------------------------- | -------------------------------------------------------------------------- |
| **Live streaming** (UnboundedRange)    | Stellar network peers | TCP outbound on varied ports (11625 default, 11725 SDF pubnet, others) | After initial catchup, Captive Core connects to peers for live ledger data |
| **Historical backfill** (BoundedRange) | History archives      | HTTPS/443 outbound                                                     | Captive Core reads from SDF history archives only, no peer connections     |
| **Both modes**                         | S3                    | HTTPS/443 via VPC endpoint                                             | Write exported XDR artifacts                                               |

Since the live ingestion task requires peer connectivity and the backfill task requires HTTPS to archives, and both write to S3, the simplest approach is a single VPC setup that supports all three paths.

### Option A: Private Subnet + NAT Gateway (Recommended)

Source: [Connect Amazon ECS applications to the internet | AWS docs](https://docs.aws.amazon.com/AmazonECS/latest/developerguide/networking-outbound.html)

```
ECS Fargate Task (private subnet)
  → Route table → NAT Gateway (public subnet)
    → Internet Gateway → Stellar peers (any IP, varied TCP ports)
  → Route table → S3 Gateway Endpoint → S3 (no internet traversal)
```

This is the security-preferred architecture. The Fargate task has no public IP, cannot be reached from the internet, but can initiate outbound connections.

**NAT Gateway requirements:**

- Deploy one NAT Gateway per Availability Zone used (for HA)
- Attach an Elastic IP to each NAT Gateway
- The NAT Gateway lives in a **public** subnet; Fargate tasks live in **private** subnets
- Outbound to Stellar peers on varied TCP ports (11625, 11725, others) must be allowed in the task security group egress rules

**Cost note:** NAT Gateway charges per-GB data processed. Routing S3 traffic through a Gateway Endpoint avoids NAT charges for S3 writes.

### S3 VPC Gateway Endpoint

Source: [Best practices for connecting Amazon ECS to AWS services from inside your VPC | AWS docs](https://docs.aws.amazon.com/AmazonECS/latest/developerguide/networking-connecting-vpc.html)

- **Gateway endpoints are free** — no hourly or data-processing charge
- Attach the S3 Gateway endpoint to the route table of the private subnets used by Fargate
- This routes `s3://` traffic within the AWS network, bypassing the NAT Gateway and internet

```
Endpoint type: Gateway
Service: com.amazonaws.<region>.s3
Route tables: [private subnet route tables]
```

**Required for Fargate tasks pulling from ECR too:** ECR image layers are stored in S3. Without an S3 Gateway endpoint, ECR image pulls from a private subnet must traverse the NAT Gateway, adding cost and latency. With the endpoint, ECR pulls are free.

### Additional VPC Endpoints for Fully Private Fargate

> **Source**: VPC endpoint service names confirmed in `aws-docs-privatelink-services.md`.

If you want Fargate tasks to run with **no NAT Gateway at all** (fully isolated VPC), you need these Interface endpoints (PrivateLink, charged hourly):

| Service                          | Endpoint type | Required for                   |
| -------------------------------- | ------------- | ------------------------------ |
| `com.amazonaws.<region>.ecr.api` | Interface     | ECR authentication             |
| `com.amazonaws.<region>.ecr.dkr` | Interface     | ECR image pull                 |
| `com.amazonaws.<region>.s3`      | Gateway       | S3 (free)                      |
| `com.amazonaws.<region>.logs`    | Interface     | CloudWatch Logs                |
| `com.amazonaws.<region>.ssm`     | Interface     | SSM Parameter Store (optional) |

**Important:** A fully isolated VPC (no NAT) cannot reach **any** external endpoints on the public internet. This affects Galexie in **both** modes:

- **Live streaming**: Cannot reach Stellar peers on TCP (ports 11625, 11725, etc.).
- **Backfill**: Cannot reach history archives at `history.stellar.org` (HTTPS/443) — these are public internet endpoints, not AWS services.

Therefore, NAT Gateway (or a public subnet) is required for **all** Galexie deployment modes, not just live streaming.

### Recommended Architecture (Single-AZ: us-east-1a)

Per the project's infrastructure overview, the deployment uses a **single-AZ topology in us-east-1a**. This simplifies NAT Gateway to a single instance (no cross-AZ redundancy needed).

```
VPC (us-east-1)
├── Public Subnet (us-east-1a)
│   └── NAT Gateway (1x, with Elastic IP)
├── Private Subnet (us-east-1a)
│   └── ECS Fargate Tasks (Galexie + Captive Core)
├── Route Table (private subnet)
│   ├── 0.0.0.0/0 → NAT Gateway  (for Stellar peers + history archives)
│   └── pl-<s3-prefix-list> → S3 Gateway Endpoint
└── Security Group (Fargate tasks)
    ├── Egress: All TCP → 0.0.0.0/0    (peers on 11625/11725/other + HTTPS for archives/ECR/CW)
    └── Ingress: (none required — Captive Core is outbound-only for Galexie)
```

> **Cost note**: Single NAT Gateway = ~$32/month base ($0.045/hr × 720h, confirmed in `aws-vpc-pricing.md`) + $0.045/GB data processing. S3 Gateway Endpoint is free and bypasses NAT for S3 writes.

---

## 4. ECS Fargate Constraints Affecting Galexie

### CPU and Memory Valid Combinations

Source: [Troubleshoot invalid CPU or memory errors | AWS docs](https://docs.aws.amazon.com/AmazonECS/latest/developerguide/task-cpu-memory-error.html)

Full table of valid Fargate task-level CPU + memory combinations:

| CPU value       | Memory range            | OS             |
| --------------- | ----------------------- | -------------- |
| 256 (.25 vCPU)  | 512 MiB, 1 GB, 2 GB     | Linux only     |
| 512 (.5 vCPU)   | 1–4 GB                  | Linux only     |
| 1024 (1 vCPU)   | 2–8 GB                  | Linux, Windows |
| 2048 (2 vCPU)   | 4–16 GB in 1 GB steps   | Linux, Windows |
| 4096 (4 vCPU)   | 8–30 GB in 1 GB steps   | Linux, Windows |
| 8192 (8 vCPU)   | 16–60 GB in 4 GB steps  | Linux only     |
| 16384 (16 vCPU) | 32–120 GB in 8 GB steps | Linux only     |

8 vCPU and 16 vCPU require Linux platform version 1.4.0+.

### Ephemeral Storage Is Not Shared Across Tasks

Each Fargate task gets its own isolated ephemeral storage. There is no way to share bucket files between Galexie task instances — each instance maintains its own Captive Core state. This is by design; Galexie is built to run as parallel independent instances.

### No Persistent Block Storage (EBS) on Fargate

Fargate does not support attaching EBS volumes directly (unlike EC2). All local storage is ephemeral. Options for persistence:

- Amazon EFS (NFS-based, adds latency)
- Accept ephemeral bucket storage with longer cold starts after task restarts

### Network Mode

Fargate tasks use `awsvpc` network mode exclusively. Each task gets its own ENI and private IP address. Security groups attach at the task level, not the host level.

### Platform Version

Always specify `LATEST` (or explicitly `1.4.0`) to get:

- Configurable ephemeral storage (up to 200 GiB)
- Encrypted ephemeral storage
- Task metadata endpoint v4 for storage metrics

### Fargate Does Not Support `host` or `bridge` Network Modes

Captive Core's default peer port (11625) is bound inside the container. It does not need to be exposed externally for Galexie's use case (Captive Core is an embedded subprocess, not a standalone node accepting inbound peer connections for consensus).

---

## 5. Recommended Fargate Task Definition Settings

```json
{
  "family": "galexie-captive-core",
  "requiresCompatibilities": ["FARGATE"],
  "networkMode": "awsvpc",
  "cpu": "4096",
  "memory": "16384",
  "ephemeralStorage": {
    "sizeInGiB": 100
  },
  "executionRoleArn": "arn:aws:iam::<account>:role/ecsTaskExecutionRole",
  "taskRoleArn": "arn:aws:iam::<account>:role/galexie-task-role",
  "containerDefinitions": [
    {
      "name": "galexie",
      "image": "<ecr-repo>/galexie:v25.1.1",
      "essential": true,
      "command": [
        "append",
        "--start",
        "50457424",
        "--config-file",
        "/config/config.toml"
      ],
      "environment": [
        {
          "name": "CAPTIVE_CORE_STORAGE_PATH",
          "value": "/data/captive-core"
        }
      ],
      "logConfiguration": {
        "logDriver": "awslogs",
        "options": {
          "awslogs-group": "/ecs/galexie",
          "awslogs-region": "<region>",
          "awslogs-stream-prefix": "ecs"
        }
      }
    }
  ]
}
```

### Task IAM Role Permissions (S3)

The `galexie-task-role` needs these S3 permissions (from Galexie admin guide):

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Sid": "AllowS3BucketOperations",
      "Effect": "Allow",
      "Action": [
        "s3:ListBucket",
        "s3:GetBucketLocation",
        "s3:ListBucketMultipartUploads"
      ],
      "Resource": "arn:aws:s3:::my-galexie-bucket"
    },
    {
      "Sid": "AllowS3ObjectAccess",
      "Effect": "Allow",
      "Action": [
        "s3:PutObject",
        "s3:GetObject",
        "s3:DeleteObject",
        "s3:AbortMultipartUpload",
        "s3:ListMultipartUploadParts"
      ],
      "Resource": ["arn:aws:s3:::my-galexie-bucket/*"]
    }
  ]
}
```

Use ECS task role (not EC2 instance profile) — Fargate picks up IAM credentials from the task role via the ECS credentials endpoint.

### Config File Injection

The task definition above references `/config/config.toml` but doesn't show how it gets into the container. Options for injecting the Galexie config into a Fargate task:

| Method                      | How                                                                                        | Pros                                     | Cons                                    |
| --------------------------- | ------------------------------------------------------------------------------------------ | ---------------------------------------- | --------------------------------------- |
| **Build into custom image** | `COPY config.toml /config/config.toml` in Dockerfile                                       | Simplest, no runtime dependencies        | Requires image rebuild on config change |
| **S3 download at startup**  | Custom entrypoint script: `aws s3 cp s3://bucket/config.toml /config/ && exec galexie ...` | Config changes without image rebuild     | Requires S3 access + custom entrypoint  |
| **AWS SSM Parameter Store** | Store TOML as SSM parameter, download in entrypoint                                        | Centralized config management, versioned | Requires SSM VPC endpoint or NAT        |
| **EFS volume mount**        | Mount EFS filesystem with config file                                                      | Shared config across tasks               | Adds EFS dependency and latency         |

For simplicity, **building config into a custom image** (extending `stellar/stellar-galexie`) is recommended for initial deployment. Switch to S3 or SSM when config needs to change independently of deployments.

---

## 6. Performance Context

From the [Galexie introduction blog post](https://stellar.org/blog/developers/introducing-galexie-efficiently-extract-and-store-stellar-data):

- Single Galexie instance: ~150 days for full historical backfill
- 40+ parallel instances: full 10-year mainnet backfill in under 5 days for ~$600
- Complete Stellar mainnet history: ~3 TB in cloud storage
- Ongoing operational cost: ~$160/month (source: `stellar-blog-introducing-galexie.md`; breakdown not specified in source)

For ongoing streaming (not backfill), a single Galexie instance is sufficient and can keep up with the live network.

---

## 7. Open Questions

- Does the 100 GB ephemeral storage meaningfully slow cold starts for Captive Core? (bucket files need to be re-fetched from history archives on each cold start)
- Should EFS be used to persist bucket files and reduce cold-start time for short-lived tasks?
- Is 16 GB RAM sufficient for mainnet with BucketListDB, or does the in-memory state grow beyond that for certain ledger ranges?
- What is the Captive Core startup time (history archive replay) on a cold Fargate task?
- **IOPS**: Does Fargate ephemeral SSD meet the 5K IOPS requirement in practice? AWS does not publish guarantees. Needs empirical testing.

---

## Sources

- [Galexie Prerequisites | Stellar Docs](https://developers.stellar.org/docs/data/indexers/build-your-own/galexie/admin_guide/prerequisites)
- [Galexie Setup | Stellar Docs](https://developers.stellar.org/docs/data/indexers/build-your-own/galexie/admin_guide/setup)
- [Stellar Core Prerequisites (validator) | Stellar Docs](https://developers.stellar.org/docs/validators/admin-guide/prerequisites)
- [Introducing Galexie | Stellar Blog](https://stellar.org/blog/developers/introducing-galexie-efficiently-extract-and-store-stellar-data)
- [stellar/stellar-galexie | GitHub](https://github.com/stellar/stellar-galexie)
- [Fargate task ephemeral storage | AWS docs](https://docs.aws.amazon.com/AmazonECS/latest/developerguide/fargate-task-storage.html)
- [Troubleshoot invalid CPU or memory | AWS docs](https://docs.aws.amazon.com/AmazonECS/latest/developerguide/task-cpu-memory-error.html)
- [Connect ECS applications to the internet | AWS docs](https://docs.aws.amazon.com/AmazonECS/latest/developerguide/networking-outbound.html)
- [Best practices for connecting ECS to AWS services from VPC | AWS docs](https://docs.aws.amazon.com/AmazonECS/latest/developerguide/networking-connecting-vpc.html)
- [ledgerbackend package (CaptiveStellarCore) | pkg.go.dev](https://pkg.go.dev/github.com/stellar/go/ingest/ledgerbackend) — BoundedRange vs UnboundedRange networking modes
