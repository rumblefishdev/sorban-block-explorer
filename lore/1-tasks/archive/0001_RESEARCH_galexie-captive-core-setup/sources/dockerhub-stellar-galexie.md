---
url: 'https://hub.docker.com/r/stellar/stellar-galexie'
title: 'stellar/stellar-galexie - Docker Image'
fetched_date: 2026-03-25
task_id: '0001'
---

# stellar/stellar-galexie - Docker Image

**Source:** https://hub.docker.com/r/stellar/stellar-galexie
**Fetched:** 2026-03-25

> Note: Docker Hub renders as a JavaScript SPA and is not directly machine-readable. This file combines the Docker Hub image reference with official documentation from the Stellar Galexie GitHub repository and Stellar developer docs.

---

## Docker Pull Command

```bash
docker pull stellar/stellar-galexie
```

## What is Galexie?

Galexie is a straightforward, lightweight application designed to aggregate Stellar network data, process it, and export it to an external data repository. It forms the foundation of the Composable Data Pipeline (CDP) and serves as the initial step in making raw Stellar ledger metadata accessible.

**GitHub repository:** https://github.com/stellar/stellar-galexie

## Key Features

- Exporting Stellar ledger metadata to cloud storage (GCS or S3)
- Configurable to export a specified ledger range or continuously stream new ledgers
- Exports ledger metadata in XDR (Stellar Core's native format)
- Compresses data before export to optimize storage efficiency
- Publishes metrics through an HTTP admin endpoint in Prometheus format

## Hardware Requirements

Minimum requirements to run the Galexie container:

| Resource        | Minimum                      |
| --------------- | ---------------------------- |
| RAM             | 16 GB                        |
| CPU             | 4 vCPUs                      |
| Persistent Disk | 100 GB with at least 5K IOPS |

## Running Galexie with Docker

### Basic run (local usage)

```bash
docker run stellar/stellar-galexie append \
  --start <ledger_sequence> \
  --config-file /path/to/config.toml
```

### Example: GCP Compute Engine deployment

```bash
gcloud compute instances create-with-container galexie-instance \
  --scopes=cloud-platform \
  --machine-type=e2-medium \
  --container-image=stellar/stellar-galexie \
  --disk=name=galexie-config-disk,device-name=galexie-config-disk,mode=ro,auto-delete=no \
  --container-mount-disk=mount-path=/mnt/config,mode=ro,name=galexie-config-disk \
  --container-arg="append" \
  --container-arg="--start" \
  --container-arg="1554952" \
  --container-arg="--config-file" \
  --container-arg="/mnt/config/testnet-config.toml"
```

### Example: Quickstart with Galexie enabled

```bash
docker run -it -p 8000:8000 stellar/quickstart --local --enable core,rpc,galexie
```

## Configuration File (TOML)

### Testnet export to GCS

```toml
[datastore_config]
type = "GCS"

[datastore_config.params]
destination_bucket_path = "galexie-data/ledgers/testnet"

[datastore_config.schema]
ledgers_per_file = 1
files_per_partition = 10

[stellar_core_config]
  network = "testnet"
```

## Prerequisites

### 1. Cloud Platform Account

Galexie exports Stellar ledger metadata to Google Cloud Storage (GCS) or Amazon Simple Storage Service (S3). You need the relevant account and credentials for the cloud storage service you choose.

**GCP (GCS):**

- Permissions to create a new GCS bucket, or access to an existing bucket with read/write permissions

**AWS (S3):**

- Permissions to create a new S3 bucket, or access to an existing bucket with read/write permissions

### 2. Container Runtime (Recommended)

- Kubernetes 1.19+, or
- Any host machine (e.g. AWS EC2, GCP VM) with an OCI-compliant container runtime like Docker

> While it is possible to natively install Galexie without a container runtime, this requires manual dependency management and is recommended only for advanced users.

## Cloud Storage Setup

### AWS S3 - IAM Policy

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
      "Resource": "arn:aws:s3:::my-galexie-bucket-example"
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
      "Resource": ["arn:aws:s3:::my-galexie-bucket-example/*"]
    }
  ]
}
```

### GCS - Required IAM Permissions

The following permissions are required when using GCP IAM:

- `storage.buckets.get`
- `storage.buckets.list`
- `storage.multipartUploads.abort`
- `storage.multipartUploads.create`
- `storage.multipartUploads.list`
- `storage.multipartUploads.listParts`
- `storage.objects.create`
- `storage.objects.delete`
- `storage.objects.get`
- `storage.objects.list`
- `storage.objects.restore`
- `storage.objects.update`

## Additional Resources

- [Introduction to Galexie](https://developers.stellar.org/docs/data/indexers/build-your-own/galexie)
- [Admin Guide - Prerequisites](https://developers.stellar.org/docs/data/indexers/build-your-own/galexie/admin_guide/prerequisites)
- [Admin Guide - Setup](https://developers.stellar.org/docs/data/indexers/build-your-own/galexie/admin_guide/setup)
- [Example: Export to GCS](https://developers.stellar.org/docs/data/indexers/build-your-own/galexie/examples/gcs-export)
- [Developer Guide](https://github.com/stellar/stellar-galexie/blob/master/DEVELOPER_GUIDE.md)
