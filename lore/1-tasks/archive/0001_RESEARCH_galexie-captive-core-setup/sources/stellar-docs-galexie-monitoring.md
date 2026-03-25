---
url: 'https://developers.stellar.org/docs/data/indexers/build-your-own/galexie/admin_guide/monitoring'
title: 'Monitoring | Stellar Docs'
fetched_date: 2026-03-25
task_id: '0001'
---

# Monitoring

## Metrics

Galexie publishes metrics through an HTTP-based admin endpoint, which makes it easier to monitor its performance. The data is exposed in Prometheus format, enabling easy integration with existing monitoring and alerting systems.

The admin port where these metrics are served can be configured by setting the `admin_port` variable. By default, the `admin_port` is set to `6061`.

```toml
# Admin port configuration
# Specifies the port for hosting the HTTP service that publishes metrics.
admin_port = 6061
```

With this configuration, the URL to access the metrics endpoint will be:

```
http://<host>:6061/metrics
```

## Application-Specific Metrics

Galexie emits several application-specific metrics to help track the export process:

| Metric                                  | Description                                                     |
| --------------------------------------- | --------------------------------------------------------------- |
| `galexie_last_exported_ledger`          | The sequence number of the most recently exported ledger        |
| `galexie_uploader_put_duration_seconds` | The time taken to upload objects to the data lake               |
| `galexie_uploader_object_size_bytes`    | Compressed and uncompressed sizes of the objects being uploaded |
| `galexie_upload_queue_length`           | Number of objects currently queued and waiting to be uploaded   |

In addition to these application-specific metrics, Galexie also exports:

- System metrics (e.g., CPU, memory, open file descriptors)
- Stellar Core ingestion metrics such as `galexie_ingest_ledger_fetch_duration_seconds`

## Useful Prometheus Queries

Use these metrics to build queries that monitor Galexie's performance and export process:

- **Export Times**: Query `galexie_uploader_put_duration_seconds` to monitor average upload times.
- **Queue Length**: Use `galexie_upload_queue_length` to view the number of objects waiting to be uploaded.
- **Latest Exported Ledger**: Track `galexie_last_exported_ledger` to ensure that ledger exports are up-to-date.

## Grafana Dashboard

For a quick start, download the pre-built Grafana dashboard for Galexie at [Grafana Dashboards - Stellar Galexie (ID: 22285)](https://grafana.com/grafana/dashboards/22285-stellar-galexie/). This dashboard provides pre-configured queries and visualizations to help you monitor Galexie's health. You can customize it to fit your specific needs.

## Logging

Galexie emits logs to stdout and generates a log line for every object being exported to help monitor progress.

Example logs:

```
INFO[2024-11-07T17:40:37.795-08:00] Uploading: FFFFFF37--200-299/FFFFFF37--200.xdr.zstd  pid=98734 service=galexie
INFO[2024-11-07T17:40:37.892-08:00] Uploaded FFFFFF37--200-299/FFFFFF37--200.xdr.zstd successfully  pid=98734 service=galexie
```
