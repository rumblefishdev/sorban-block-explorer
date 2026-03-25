---
prefix: R
title: 'Galexie CLI flags, configuration, and container image'
status: developing
spawned_from: null
spawns: []
---

# Galexie CLI flags, configuration, and container image

## What is Galexie?

Galexie is a lightweight, standalone application that extracts Stellar ledger metadata via a Captive Core process, bundles it, compresses it (zstd), and writes it to an external data store (GCS, S3, or local filesystem). It is the foundation of the Composable Data Pipeline (CDP). The exported format is XDR `LedgerCloseMetaBatch` objects.

- GitHub: https://github.com/stellar/stellar-galexie
- Docs: https://developers.stellar.org/docs/data/indexers/build-your-own/galexie
- Latest release: **v25.1.1** (2026-03-25)

---

## Official Container Image

There is an **official Docker Hub image**:

```
docker pull stellar/stellar-galexie
```

Docker Hub URL: https://hub.docker.com/r/stellar/stellar-galexie

The image is a multi-stage build (Go builder + Ubuntu 24.04 runtime) that bundles:

- The `stellar-galexie` binary at `/usr/bin/stellar-galexie`
- A `stellar-core` binary installed from the SDF APT repository

**ENTRYPOINT** is `/usr/bin/stellar-galexie`, **CMD** defaults to `--help`.

No manual build is required for standard use — the official image is production-ready.

---

## Build Instructions (if building manually)

From the `stellar/stellar-galexie` repo (Dockerfile in `docker/` directory):

```dockerfile
FROM golang:1.24-bookworm AS builder
WORKDIR /go-galexie
COPY go.mod ./
COPY go.sum ./
RUN go mod download
COPY . ./
ARG GOFLAGS
RUN go install ./...

FROM ubuntu:24.04
ARG STELLAR_CORE_VERSION
ENV STELLAR_CORE_VERSION=${STELLAR_CORE_VERSION:-*}
ENV STELLAR_CORE_BINARY_PATH /usr/bin/stellar-core
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl wget gnupg apt-utils
RUN wget -qO - https://apt.stellar.org/SDF.asc | APT_KEY_DONT_WARN_ON_DANGEROUS_USAGE=true apt-key add -
RUN echo "deb https://apt.stellar.org noble stable" >/etc/apt/sources.list.d/SDF.list
RUN echo "deb https://apt.stellar.org noble unstable" >/etc/apt/sources.list.d/SDF-unstable.list
RUN apt-get update && apt-get install -y stellar-core=${STELLAR_CORE_VERSION}
RUN apt-get clean
COPY --from=builder /go/bin/stellar-galexie /usr/bin/stellar-galexie
ENTRYPOINT ["/usr/bin/stellar-galexie"]
CMD ["--help"]
```

Build via the Makefile (confirmed in `github-galexie-makefile.md`):

```bash
make docker-build
```

To test with a local fake GCS backend:

```bash
make docker-test-fake-gcs
```

---

## CLI Commands

The binary is `stellar-galexie` (or `galexie` in some docs). All commands take `--config-file` and use `--start` / `--end` ledger sequence numbers.

### `append` — Live export and backfill

Primary command. Two modes:

- **Continuous (unbounded)**: omit `--end` or set it to `0`. Exports from `--start` and keeps running as new ledgers close.
- **Fixed range**: provide both `--start` and `--end`. Exits when range is complete.

Resumes automatically from the last exported ledger if interrupted (see [R-s3-output-and-backfill.md](R-s3-output-and-backfill.md) § "Checkpoint-Aware Restart Mechanism" for implementation details).

```
stellar-galexie append --start <start_ledger> [--end <end_ledger>] [--config-file <path>]
```

| Flag                   | Required | Description                                            |
| ---------------------- | -------- | ------------------------------------------------------ |
| `--start <seq>`        | Yes      | Starting ledger sequence number                        |
| `--end <seq>`          | No       | Ending ledger sequence number (0 or omit = continuous) |
| `--config-file <path>` | No       | Path to config TOML (default: `./config.toml`)         |

Example — fixed range backfill from official docs (source: `stellar-docs-galexie-running.md`):

```bash
docker run --platform linux/amd64 -d \
  -v "$HOME/.config/gcloud/application_default_credentials.json":/.config/gcp/credentials.json:ro \
  -e GOOGLE_APPLICATION_CREDENTIALS=/.config/gcp/credentials.json \
  -v ${PWD}/config.toml:/config.toml \
  stellar/stellar-galexie \
  append --start 350000 --end 450000 --config-file config.toml
```

Example — S3 with AWS credentials (author-composed, not from official docs):

```bash
docker run --platform linux/amd64 -d \
  -e AWS_ACCESS_KEY_ID="$AWS_ACCESS_KEY_ID" \
  -e AWS_SECRET_ACCESS_KEY="$AWS_SECRET_ACCESS_KEY" \
  -e AWS_REGION="us-east-1" \
  -v ${PWD}/config.toml:/config.toml \
  stellar/stellar-galexie \
  append --start 50457424 --config-file config.toml
```

> **Note on ECS Fargate**: When running on Fargate, AWS credentials are automatically provided via the ECS task role — the ECS agent injects credentials via `169.254.170.2` endpoint (standard AWS ECS behavior, not Galexie-specific). No environment variables needed.

### `scan-and-fill` — Fill gaps

Identifies and exports only **missing** ledgers within a range. Useful after deletions or non-contiguous exports. `--end` is required.

```
stellar-galexie scan-and-fill --start <start_ledger> --end <end_ledger> [--config-file <path>]
```

```bash
docker run --platform linux/amd64 -d \
  -v "$HOME/.config/gcloud/application_default_credentials.json":/.config/gcp/credentials.json:ro \
  -e GOOGLE_APPLICATION_CREDENTIALS=/.config/gcp/credentials.json \
  -v ${PWD}/config.toml:/config.toml \
  stellar/stellar-galexie \
  scan-and-fill --start 64000 --end 68000 --config-file config.toml
```

### `replace` — Re-export existing ledgers (v24.1.0+)

Overwrites already-exported ledgers. Used when Stellar Core emits updated metadata (e.g., CAP-0067 protocol upgrades).

```
stellar-galexie replace --start <start_ledger> --end <end_ledger> [--config-file <path>]
```

### `detect-gaps` — Audit data lake (v25.0.0+)

Read-only scan that reports missing ledger sequences. Does not export or modify data.

```
stellar-galexie detect-gaps --start <start_ledger> --end <end_ledger> \
  [--config-file <path>] [--output-file <json_path>]
```

| Flag                   | Required | Description                                     |
| ---------------------- | -------- | ----------------------------------------------- |
| `--start <seq>`        | Yes      | Start of scan range                             |
| `--end <seq>`          | Yes      | End of scan range                               |
| `--config-file <path>` | No       | Path to config TOML                             |
| `--output-file <path>` | No       | Write JSON gap report to file (default: stdout) |

Example output JSON (from official Stellar docs):

```json
{
  "scan_from": 2,
  "scan_to": 200000,
  "duration_seconds": "3.42ms",
  "report": {
    "gaps": [
      { "start": 144320, "end": 144383 },
      { "start": 180000, "end": 180063 }
    ],
    "total_ledgers_found": 199871,
    "total_ledgers_missing": 128,
    "min_sequence_found": 2,
    "max_sequence_found": 200000
  }
}
```

---

## Configuration File (`config.toml`)

Default path: `./config.toml` in the working directory. Override with `--config-file`.

Condensed annotated example based on the official repo's `config/config.example.toml` (comments paraphrased for brevity; see source file for full original comments):

```toml
# Admin port configuration
# Port for HTTP service that publishes Prometheus metrics.
admin_port = 6061

# --- Datastore Configuration ---
[datastore_config]
# Supported types: "GCS", "S3", "Filesystem"
type = "GCS"

# --- GCS params ---
[datastore_config.params]
destination_bucket_path = "your-bucket-name/<optional_subpath1>/<optional_subpath2>/"

# --- S3 / S3-compatible params (commented out by default) ---
# destination_bucket_path = "your-bucket-name/<optional_subpath1>/<optional_subpath2>/"
# region = "us-west-1"
# endpoint_url = "https://00000000000000000000000000000000.cloudflarestorage.com"
# (endpoint_url enables Cloudflare R2, MinIO, DigitalOcean Spaces, etc.)

# --- Filesystem params (dev/testing only, no metadata support) ---
# destination_path = "/path/to/local/storage"

[datastore_config.schema]
ledgers_per_file = 1        # How many ledgers per exported file
files_per_partition = 64000 # Number of files per partition directory

# --- Stellar Core (Captive Core) Configuration ---
[stellar_core_config]
# Shorthand: "testnet" or "pubnet". Sets defaults for history archives and passphrase.
network = "testnet"

# Manual overrides (any of these override the 'network' shorthand):
# captive_core_toml_path = "my-captive-core.cfg"  # Path to captive core config file
# history_archive_urls = ["http://testarchiveurl1", "http://testarchiveurl2"]
# network_passphrase = "Test SDF Network ; September 2015"
# stellar_core_binary_path = "/my/path/to/stellar-core"
# (Not needed in Docker: stellar-core is pre-installed at /usr/bin/stellar-core)
```

### Key `[stellar_core_config]` fields

| Field                      | Type     | Description                                                                        |
| -------------------------- | -------- | ---------------------------------------------------------------------------------- |
| `network`                  | string   | `"testnet"` or `"pubnet"`. Sets passphrase and history archive URLs automatically. |
| `captive_core_toml_path`   | string   | Path to a custom captive core `.cfg` file. Overrides `network` defaults.           |
| `history_archive_urls`     | []string | History archive URLs. Overrides `network` defaults.                                |
| `network_passphrase`       | string   | Network passphrase. Overrides `network` defaults.                                  |
| `stellar_core_binary_path` | string   | Path to `stellar-core` binary. Not needed inside Docker.                           |

### Key `[datastore_config.schema]` fields

| Field                 | Default | Description                           |
| --------------------- | ------- | ------------------------------------- |
| `ledgers_per_file`    | 1       | Ledgers bundled in each exported file |
| `files_per_partition` | 64000   | Files per directory partition         |

---

## Monitoring

Galexie exposes Prometheus metrics on the configured `admin_port` (default 6061):

```
http://<host>:6061/metrics
```

Key metrics:

| Metric                                         | Description                                      |
| ---------------------------------------------- | ------------------------------------------------ |
| `galexie_last_exported_ledger`                 | Sequence number of most recently exported ledger |
| `galexie_uploader_put_duration_seconds`        | Upload duration histogram                        |
| `galexie_uploader_object_size_bytes`           | Compressed and uncompressed object sizes         |
| `galexie_upload_queue_length`                  | Objects waiting to be uploaded                   |
| `galexie_ingest_ledger_fetch_duration_seconds` | Captive Core fetch duration                      |

A pre-built Grafana dashboard is available at: https://grafana.com/grafana/dashboards/22285-stellar-galexie/

---

## Hardware Requirements

From the official Prerequisites page:

| Resource        | Minimum                 |
| --------------- | ----------------------- |
| RAM             | 16 GB                   |
| CPU             | 4 vCPUs                 |
| Persistent Disk | 100 GB, 5K IOPS minimum |

---

## Full History Export Strategy

- Full history is ~3 TB stored, ~$1,100 total cost on GCP (compute + GCS writes)
- Single instance: ~150 days
- 40-50 parallel instances: ~4-5 days
- Earlier ledgers (< ~30,000,000) are smaller and faster to export
- Recommended split for 50 instances: 15 for genesis–29,999,999 and 35 for 30,000,000 onward

Parallel local invocation example:

```bash
./galexie append --start 2 --end 1999999 & \
./galexie append --start 2000000 --end 3999999 & \
./galexie append --start 4000000 --end 5999999 &
```

---

## GCS Example: Minimal testnet config

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

Run continuous append from ledger 1554952:

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

> **Note on machine type**: The official GCS export page states that `e2-medium` (2 vCPU, 4 GB RAM) "satisfies Galexie Prerequisites" for this testnet example. However, the separate Prerequisites page lists minimum requirements as 4 vCPU / 16 GB / 100 GB disk. This is an inconsistency in the official Stellar documentation. For mainnet production use, follow the Prerequisites page (4 vCPU / 16 GB minimum).

---

## Sources

- https://github.com/stellar/stellar-galexie
- https://developers.stellar.org/docs/data/indexers/build-your-own/galexie
- https://developers.stellar.org/docs/data/indexers/build-your-own/galexie/admin_guide/running
- https://developers.stellar.org/docs/data/indexers/build-your-own/galexie/admin_guide/prerequisites
- https://developers.stellar.org/docs/data/indexers/build-your-own/galexie/admin_guide/full-history-exporting
- https://developers.stellar.org/docs/data/indexers/build-your-own/galexie/admin_guide/monitoring
- https://developers.stellar.org/docs/data/indexers/build-your-own/galexie/examples/gcs-export
- https://hub.docker.com/r/stellar/stellar-galexie
- https://stellar.org/blog/developers/introducing-galexie-efficiently-extract-and-store-stellar-data
