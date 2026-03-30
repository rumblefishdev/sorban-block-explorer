---
id: '0001'
title: 'Research: Galexie configuration, Captive Core setup, and output format'
type: RESEARCH
status: completed
related_adr: []
related_tasks: ['0024', '0034']
tags: [priority-high, effort-medium, layer-research]
milestone: 1
links:
  - https://github.com/stellar/stellar-galexie
  - https://developers.stellar.org/docs/data/indexers/build-your-own/galexie
  - https://hub.docker.com/r/stellar/stellar-galexie
history:
  - date: 2026-03-24
    status: backlog
    who: fmazur
    note: 'Task created from architecture docs decomposition'
  - date: 2026-03-25
    status: active
    who: fmazur
    note: 'Research started - investigating Galexie CLI, Captive Core, S3 output, ECS sizing'
  - date: 2026-03-25
    status: completed
    who: fmazur
    note: >
      Research complete. 4 notes, 30 archived sources, 9/9 acceptance criteria met.
      Key findings: official Docker image stellar/stellar-galexie, BoundedRange vs
      UnboundedRange networking, hex-prefixed S3 keys with .xdr.zst, Soroban activation
      ledger 50457424, Fargate 4vCPU/16GB/100GiB sizing. 3 corrections to architecture
      docs identified (Soroban ledger, S3 key format, peer connectivity model).
---

# Research: Galexie configuration, Captive Core setup, and output format

## Summary

Investigate the configuration, deployment, and operational characteristics of Galexie running on ECS Fargate with Captive Core for live ledger export and historical backfill.

## Status: Completed

## Research Notes

| Note                                                             | Topic                                                              |
| ---------------------------------------------------------------- | ------------------------------------------------------------------ |
| [R-galexie-cli-and-image.md](notes/R-galexie-cli-and-image.md)   | CLI flags, config TOML, Docker image, build instructions           |
| [R-captive-core-config.md](notes/R-captive-core-config.md)       | Captive Core TOML templates, network passphrases, peer behavior    |
| [R-s3-output-and-backfill.md](notes/R-s3-output-and-backfill.md) | S3 key format, compression, checkpoint restart, backfill model     |
| [R-ecs-fargate-sizing.md](notes/R-ecs-fargate-sizing.md)         | Fargate CPU/memory/storage sizing, VPC networking, task definition |

## Key Findings

### Galexie CLI & Image

- Official Docker image: `stellar/stellar-galexie` (Docker Hub), bundles stellar-core
- 4 subcommands: `append` (live+backfill), `scan-and-fill` (gaps), `replace` (overwrite), `detect-gaps` (audit)
- Config via TOML: `[datastore_config]` (S3/GCS/FS), `[stellar_core_config]` (network shorthand or explicit)
- Latest release: v25.1.1

### Captive Core

- Network passphrases: mainnet=`Public Global Stellar Network ; September 2015`, testnet=`Test SDF Network ; September 2015`
- Galexie config shorthand: `network = "pubnet"` or `network = "testnet"` auto-configures passphrase + archives
- BucketListDB is official primary backend since August 2024. Use `CAPTIVE_CORE_STORAGE_PATH`, NOT `BUCKET_DIR_PATH`
- **Two networking modes**: BoundedRange (backfill) uses history archives only (HTTPS). UnboundedRange (live) catches up via archives, then connects to Stellar peers (TCP on varied ports: 11625, 11725, others)
- **Peer failure handling**: stellar-core overlay manager reconnects automatically. If subprocess exits, Galexie terminates — relies on ECS service scheduler to restart. Resume is lossless via checkpoint mechanism.
- **Inbound connections**: NOT needed for Galexie (Captive Core is embedded subprocess, not a standalone peer node)

### S3 Output Format

- **XDR content**: Each file is a `LedgerCloseMetaBatch` containing 1+ `LedgerCloseMeta` entries (count = `ledgers_per_file`). Lambda must parse as batch, not raw `LedgerCloseMeta`.
- Key pattern: `{hex(0xFFFFFFFF-partition_start)}--{part_start}-{part_end}/{hex(0xFFFFFFFF-file_start)}--{file_start}[-{file_end}].xdr.zst`
- Compression: zstd only (hardcoded since v23.0)
- `ledgers_per_file` configurable (1 = public data lake default, 64 = 2x faster bulk downloads)
- `files_per_partition` default: 64000

### Checkpoint Restart

- `append` mode scans S3 for first missing ledger at/after `--start`, resumes from there
- Alignment to `LedgersPerFile` boundary enforced; misalignment -> error suggesting `scan-and-fill`
- No separate state store; queries datastore directly

### Backfill Model

- No built-in parallelism flag; run multiple processes on non-overlapping ranges
- SDF backfilled 10 years with 40+ parallel instances in <5 days (~$600)
- Live + backfill coordination: implicit via idempotent file writes; each process skips present files

### Soroban Activation Ledger

- **50,457,424** (Protocol 20, February 20, 2024) - corrected from architecture docs' ~50,692,993. Source: community blog post, not official SDF docs. Should be cross-verified against a full-archive Horizon instance before hardcoding.

### ECS Fargate Sizing

- CPU: 4096 (4 vCPU), Memory: 16384 MiB (16 GB) minimum
- Ephemeral storage: 100 GiB (platform 1.4.0+). **IOPS caveat**: Galexie requires 5K IOPS but AWS doesn't guarantee IOPS on Fargate ephemeral storage
- VPC (single-AZ us-east-1a): S3 Gateway Endpoint (free) + 1x NAT Gateway for outbound (peers + archives)
- Task role needs s3:PutObject, s3:GetObject, s3:ListBucket on the ledger bucket

## Acceptance Criteria

- [x] Documented Galexie CLI flags and configuration options for both live and backfill modes
- [x] Container image source and build instructions (or official image reference)
- [x] Captive Core configuration template with network passphrase for mainnet and testnet
- [x] ECS Fargate task definition sizing recommendations (CPU, memory, storage)
- [x] VPC networking requirements documented: S3 VPC endpoint, NAT Gateway for peer connectivity
- [x] S3 output format contract validated: key pattern, compression, single vs batched ledgers
- [x] Checkpoint restart behavior confirmed and documented
- [x] Backfill execution model documented: ledger range configuration, parallelism, history archive URL
- [x] Confirmed Soroban mainnet activation ledger number for backfill scoping

## Corrections to Architecture Docs

1. **Soroban activation ledger**: Architecture docs say ~50,692,993. Actual Protocol 20 activation is **50,457,424** (Feb 20, 2024). Needs cross-verification against full-archive Horizon before hardcoding.
2. **S3 key format**: Architecture docs describe `stellar-ledger-data/ledgers/{seq_start}-{seq_end}.xdr.zstd`. Actual format uses hex-prefixed partition directories and `.xdr.zst` extension (not `.xdr.zstd`).
3. **Peer connectivity**: Architecture docs imply always-on P2P. In reality, networking depends on mode: **backfill** (BoundedRange) uses history archives over HTTPS only; **live streaming** (UnboundedRange) catches up via archives then connects to Stellar peers on varied TCP ports (11625, 11725, others). NAT Gateway needed for both modes because history archives (`history.stellar.org`) are on the public internet, not AWS services.

## Caveats & Unverified Assumptions

- **Soroban ledger number**: 50,457,424 from community source, not official SDF docs. Cross-verify before production use.
- **Fargate IOPS**: 5K IOPS requirement vs no AWS guarantees on ephemeral storage. Likely fine but unproven.
- **`[buffered_storage_backend_config]`**: This section does NOT exist in Galexie — it belongs to Stellar RPC. Do not include in Galexie config.
- **Config key**: The correct Galexie TOML key for custom Captive Core config is `captive_core_toml_path` (not `captive_core_config_path`).

## Research Questions → Answer Location

| #   | Question                                             | Answered in                                                                                  |
| --- | ---------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| 1   | Galexie CLI flags and config options                 | R-galexie-cli-and-image.md § "CLI Commands" + § "Configuration File"                         |
| 2   | Container image (official or build from source?)     | R-galexie-cli-and-image.md § "Official Container Image" + § "Build Instructions"             |
| 3   | Captive Core config + network passphrases            | R-captive-core-config.md § "Network Passphrases" + § "Captive Core TOML / CFG Configuration" |
| 4   | ECS Fargate CPU/memory requirements                  | R-ecs-fargate-sizing.md § "Compute Requirements" + § "Recommended Fargate Task Definition"   |
| 5   | Peer connection failures and reconnection            | R-captive-core-config.md § "Peer connection failure and reconnection (live mode)"            |
| 6   | Exact S3 key format, single vs batched               | R-s3-output-and-backfill.md § "S3 Key Format" + § "Single Ledger vs. Batched"                |
| 7   | Checkpoint-aware restart mechanism                   | R-s3-output-and-backfill.md § "Checkpoint-Aware Restart Mechanism"                           |
| 8   | Backfill CLI flags, parallelism, history archive URL | R-s3-output-and-backfill.md § "CLI Commands and Backfill Flags"                              |
| 9   | Soroban mainnet activation ledger number             | R-s3-output-and-backfill.md § "Soroban Mainnet Activation Ledger"                            |
| 10  | Backfill + live coordination (duplicate avoidance)   | R-s3-output-and-backfill.md § "Backfill + Live Ingestion Coordination"                       |

## Notes

- Single-AZ topology (us-east-1a) is compatible - one NAT Gateway suffices
- NAT Gateway required for ALL modes (history archives are public internet, not AWS services)
- Open-source redeployability confirmed - no hard-coded account IDs in Galexie
- Failed ledger files remain absent (not corrupted) and can be re-exported via `scan-and-fill`
