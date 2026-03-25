---
url: 'https://developers.stellar.org/docs/data/indexers/build-your-own/galexie/admin_guide/running'
title: 'Running Galexie | Stellar Docs'
fetched_date: 2026-03-25
task_id: '0001'
---

# Running Galexie

## Overview

The commands and arguments in this document can be configured in the helm chart or run on an instance. The helm chart manages the config file path internally, so the `--config-file` argument should not be added to the arguments variable in the helm chart.

With the Docker image and configuration file prepared, you're ready to run Galexie and export Stellar ledger data to your storage bucket.

## Command Line Usage

### Append Command

This is the primary way to run Galexie. The `append` command operates in two modes:

- **Continuous/unbounded mode**: Starts exporting from a specified ledger and continuously exports new ledgers until interrupted.
- **Fixed range mode**: Exports a specified ledger range and exits when complete.

**Syntax:**

```
stellar-galexie append --start <start_ledger> [--end <end_ledger>] [--config-file <config_file>]
```

**Arguments:**

`--start <start_ledger>` (required)

- The starting ledger sequence number of the range being exported.

`--end <end_ledger>` (optional)

- The ending ledger sequence number. If unspecified or set to 0, the exporter continuously exports new ledgers as they appear on the network.

`--config-file <config_file_path>` (optional)

- The path to the configuration file. If unspecified, the application looks for `config.toml` in the current directory.

**Example usage:**

```bash
docker run --platform linux/amd64 -d \
-v "$HOME/.config/gcloud/application_default_credentials.json":/.config/gcp/credentials.json:ro \
-e GOOGLE_APPLICATION_CREDENTIALS=/.config/gcp/credentials.json \
-v ${PWD}/config.toml:/config.toml \
stellar/stellar-galexie \
append --start 350000 --end 450000 --config-file config.toml
```

**Docker flags explained:**

- `--platform linux/amd64` - Specifies the platform architecture (adjust if needed for your system).
- `-v` - Mounts volumes to map your local GCP credentials and config.toml file to the container:
  - `$HOME/.config/gcloud/application_default_credentials.json`: Your local GCP credentials file.
  - `${PWD}/config.toml`: Your local configuration file.
- `-e GOOGLE_APPLICATION_CREDENTIALS=/.config/gcp/credentials.json` - Sets the environment variable for credentials within the container. Use AWS equivalent if using S3 as your cloud storage service.
- `stellar/stellar-galexie` - The Docker image name.

#### Data Integrity and Resumability

The append command maintains strict sequential integrity within each export session. If interrupted and restarted with the same range, it automatically resumes from where it left off, ensuring no ledgers are missed within a session.

---

### Scan-and-fill Command

The `scan-and-fill` command identifies and fills gaps in exported ledgers. It scans all ledgers in a specified range, identifies missing ledgers, and exports only the missing ones while skipping existing ledgers.

Gaps may occur due to:

- Manual deletion of ledgers from the data lake (e.g., deleting ledgers 80-90 from range 1-100).
- Running non-contiguous export ranges (e.g., exporting 1-50 and 60-100, leaving a gap between 50-60). Running `append` with range 1-500 causes Galexie to resume from 101 without filling the gap.

**Syntax:**

```
stellar-galexie scan-and-fill --start <start_ledger> --end <end_ledger> [--config-file <config_file>]
```

**Arguments:**

`--start <start_ledger>` (required)

- The starting ledger sequence number of the range being exported.

`--end <end_ledger>` (required)

- The ending ledger sequence number of the range being exported.

`--config-file <config_file_path>` (optional)

- The path to the configuration file. If unspecified, the exporter looks for "config.toml" in the current directory.

**Example usage:**

```bash
docker run --platform linux/amd64 -d \
-v "$HOME/.config/gcloud/application_default_credentials.json":/.config/gcp/credentials.json:ro \
-e GOOGLE_APPLICATION_CREDENTIALS=/.config/gcp/credentials.json \
-v ${PWD}/config.toml:/config.toml \
stellar/stellar-galexie \
scan-and-fill --start 64000 --end 68000 --config-file config.toml
```

---

### Replace Command

The `replace` command (introduced in Galexie v24.1.0) simplifies re-exporting previously processed ledgers.

Unlike `append` or `scan-and-fill`, which skip existing files, `replace` overwrites existing files within a specified range. It is primarily used when Stellar Core emits new or updated metadata for previously processed ledgers (e.g., the introduction of [CAP-0067](https://github.com/stellar/stellar-protocol/blob/master/core/cap-0067.md) Stellar Events), allowing operators to re-export affected ledgers to ensure the data lake contains the latest metadata.

**Syntax:**

```
stellar-galexie replace --start <start_ledger> --end <end_ledger> [--config-file <config_file>]
```

**Arguments:**

`--start <start_ledger>` (required)

- The starting ledger sequence number of the range being exported.

`--end <end_ledger>` (required)

- The ending ledger sequence number of the range being exported.

`--config-file <config_file_path>` (optional)

- The path to the configuration file. If unspecified, the exporter looks for "config.toml" in the current directory.

**Example usage:**

```bash
docker run --platform linux/amd64 -d \
-v "$HOME/.config/gcloud/application_default_credentials.json":/.config/gcp/credentials.json:ro \
-e GOOGLE_APPLICATION_CREDENTIALS=/.config/gcp/credentials.json \
-v ${PWD}/config.toml:/config.toml \
stellar/stellar-galexie \
replace --start 64000 --end 68000 --config-file config.toml
```

---

### Detect-gaps Command

The `detect-gaps` command (introduced in Galexie v25.0.0) performs a read-only audit of the data lake and reports any missing ledger sequences within a given range.

Under normal operation, the `append` command maintains strict sequential integrity. However, operators commonly parallelize initial full-history exports by splitting the ledger range into multiple subranges and running many Galexie instances concurrently. Misconfigured ranges, failed jobs, or manual intervention can leave holes in the datastore.

The `detect-gaps` command verifies completeness of a newly created data lake after initial full-history export or as a periodic audit. This command does not export or modify any data; it only scans the existing datastore and reports missing ranges.

**Syntax:**

```
stellar-galexie detect-gaps --start <start_ledger> \
--end <end_ledger> \
[--config-file <config_file>] \
[--output-file <output_file>]
```

**Arguments:**

`--start <start_ledger>` (required)

- The starting ledger sequence number of the range to be scanned for gaps.

`--end <end_ledger>` (required)

- The ending ledger sequence number of the range to be scanned for gaps.

`--config-file <config_file_path>` (optional)

- The path to the configuration file. If unspecified, the application looks for "config.toml" in the current directory.

`--output-file <output_file_path>` (optional)

- If provided, the gap report is written as JSON to this file. If omitted, the JSON report is written to standard output.

**Example usage:**

```bash
docker run --platform linux/amd64 -d \
-v "$HOME/.config/gcloud/application_default_credentials.json":/.config/gcp/credentials.json:ro \
-e GOOGLE_APPLICATION_CREDENTIALS=/.config/gcp/credentials.json \
-v ${PWD}/config.toml:/config.toml \
-v ${PWD}:/reports \
stellar/stellar-galexie \
detect-gaps \
--start 2 \
--end 200000 \
--config-file config.toml \
--output-file gaps_report.json
```

**Example Output:**

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
