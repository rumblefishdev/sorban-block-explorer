---
url: 'https://developers.stellar.org/docs/data/indexers/build-your-own/galexie/examples/gcs-export'
title: 'Export to GCS | Stellar Docs'
fetched_date: 2026-03-25
task_id: '0001'
---

# Export to GCS

## Goals

- Ledger metadata is stored on Google Cloud Storage (GCS).
- Downstream consumers need access to the latest network data with minimal latency.
  - Ledger metadata for each newly closed ledger on Stellar Testnet should be expediently exported to GCS.
- Deployment shall be fully on-cloud, use GCP for all storage and compute needs.

## Solution - Publisher Pipeline

Run the Galexie Dockerhub image, [stellar/stellar-galexie](https://hub.docker.com/r/stellar/stellar-galexie), as an instance in [GCP Compute Engines](https://cloud.google.com/run/docs/create-jobs) and export the ledger metadata to a [GCS bucket](https://cloud.google.com/storage/docs/json_api/v1/buckets).

In this example, Galexie acts as both the `origin` and `publisher` of ledger metadata to the Google Cloud Storage bucket, which is the `sink`.

## Step-by-Step Setup

### 1. Prepare the Galexie Configuration File Locally

Create `testnet-config.toml`:

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

### 2. Set Default Zone and Project on gcloud

Do this once so you don't have to repeat it on all further commands. All resources created will be in the same GCP project and zone.

```bash
gcloud config set compute/zone {your zone here}
gcloud config set project {your GCP project name}
```

### 3. Store the Galexie Configuration File on a Compute Disk

Create a new GCP Compute disk to hold the configuration file for Galexie. It will be used as a volume mount for the Galexie container.

```bash
# Create the raw disk in GCP project
gcloud compute disks create galexie-config-disk \
  --size=10GB \
  --type=pd-standard

# Create a temp instance and attach the new galexie disk to it
gcloud compute instances create temp-instance \
  --machine-type=e2-medium \
  --disk=name=galexie-config-disk,device-name=galexie-config-disk,mode=rw,auto-delete=no

# Shell into the temp instance
gcloud compute ssh temp-instance

# Find the unformatted attached disk device (listed with no mountpoint and 10GB)
temp-instance:~$ lsblk

# Format the empty disk
temp-instance:~$ sudo mkfs.ext4 -F /dev/sda

# Mount the formatted disk
temp-instance:~$ sudo mkdir -p /mnt/my-disk; chmod a+rw /mnt/my-disk
temp-instance:~$ sudo mount /dev/sda /mnt/my-disk
temp-instance:~$ exit

# Copy the local testnet-config.toml file onto the formatted galexie-config-disk
gcloud compute scp testnet-config.toml temp-instance:/mnt/my-disk

# Discard the temp instance — the disk will remain
gcloud compute instances delete temp-instance
```

### 4. Create a GCS Bucket for Ledger Metadata Storage

```bash
gcloud storage buckets create gs://galexie-data
```

### 5. Deploy and Run Galexie as a Compute Instance

Configure the volume mount on the instance for Galexie to load the configuration file from the existing compute disk. Specify the starting ledger sequence for Galexie to begin exporting ledger metadata.

The starting ledger can be obtained from any block explorer, such as [stellar.expert/explorer/testnet](https://stellar.expert/explorer/testnet).

In this example the `e2-medium` machine type satisfies [Galexie Prerequisites](https://developers.stellar.org/docs/data/indexers/build-your-own/galexie/admin_guide/prerequisites).

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

### 6. Monitor Galexie Export Status

In the GCP console:

- **Cloud Storage -> Buckets**: View the contents of the `galexie-data` bucket. New files representing the latest ledger metadata from Testnet should arrive approximately every minute.
- **Compute Engine -> Virtual Machines**: Check the log output of `galexie-instance`. You'll see `level=info msg="Uploaded .."` lines indicating each time a new file of ledger metadata is uploaded.

## Next Step - Consumer Pipelines

Ledger metadata is now accumulating as files in your GCS bucket. You can start to explore options for applications to consume this pre-computed network data using the [Ingest SDK](https://developers.stellar.org/docs/data/indexers/build-your-own/ingest-sdk) to assemble consumer-driven data pipelines capable of importing and parsing the data to derive custom, enriched data models.

Refer to [GCS bucket consumer pipeline](https://developers.stellar.org/docs/build/apps/ingest-sdk/overview#ledger-metadata-consumer-pipeline) for relevant example code.
