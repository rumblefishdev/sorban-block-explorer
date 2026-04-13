# Stellar Pubnet Ledger Archive — Overview

**Source:** `s3://aws-public-blockchain/v1.1/stellar/ledgers/pubnet/`  
**Measured:** 2026-04-09

---

## Structure

The archive stores one XDR+zstd file per ledger, grouped into partition directories of 64,000 ledgers each.

| Property              | Value                                          |
| --------------------- | ---------------------------------------------- |
| Encoding              | XDR (Stellar binary)                           |
| Compression           | Zstandard (zstd)                               |
| Files per ledger      | 1                                              |
| Ledgers per partition | 64,000                                         |
| Network               | Public Global Stellar Network ; September 2015 |

File naming: `{XOR_hash}--{ledger_number}.xdr.zst`  
Partition naming: `{XOR_hash}--{start_ledger}-{end_ledger}/`

---

## Scale

| Metric                           | Value                 |
| -------------------------------- | --------------------- |
| Total partition directories      | 970                   |
| Full partitions (64,000 files)   | 969                   |
| Partial partitions (in-progress) | 1                     |
| **Total ledger files**           | **~62,039,675**       |
| Ledger range                     | 0 – 62,039,675        |
| Earliest ledger date             | ~September 2015       |
| Latest ledger date               | ~April 2026 (ongoing) |

---

## Soroban Introduction

**Soroban smart contracts** (Protocol 20) were activated on the Stellar mainnet on **February 20, 2024**.

Ledger close times were extracted from the XDR data to anchor this to the archive:

| Ledger     | Close Time (UTC) |
| ---------- | ---------------- |
| 20,992,000 | 2018-11-15       |
| 35,008,000 | 2021-04-19       |
| 46,016,000 | 2023-04-26       |
| 50,048,000 | 2024-01-23       |

At a rate of ~5.83 seconds per ledger (derived from the 46M → 50M segment), Protocol 20 activated approximately **28 days after ledger 50,048,000**, placing the activation at:

> **Soroban activation ≈ ledger 50,463,000** (2024-02-20)

This splits the archive into:

- **Pre-Soroban:** ledgers 0 – 50,463,000 (~50.5M ledgers, Sep 2015 – Feb 2024)
- **Post-Soroban:** ledgers 50,463,000 – 62,040,000 (~11.6M ledgers, Feb 2024 – present)

---

## File Size Distribution

File sizes vary dramatically across ledger history. Sizes were sampled by downloading full partition listings.

| Ledger Range  | Approx Date            | Sample Partition              | Files            | Partition Total | Avg File Size |
| ------------- | ---------------------- | ----------------------------- | ---------------- | --------------- | ------------- |
| 0 – 64k       | Sep 2015               | `FFFFFFFF--0-63999`           | 63,997           | 18 MB           | ~279 B        |
| 512k – 576k   | Oct 2015               | `FFF82FFF--512000-575999`     | 64,000           | 22 MB           | ~344 B        |
| 10M – 10.1M   | Nov 2016               | `FF66ADFF--10048000-10111999` | 64,000           | 32 MB           | ~498 B        |
| 21M – 21.1M   | Nov 2018               | `FEBFAFFF--20992000-21055999` | 64,000           | 623 MB          | ~10.2 KB      |
| 31M – 31.1M   | ~mid-2020              | `FE2757FF--30976000-31039999` | 64,000           | 1.9 GB          | ~31.5 KB      |
| 35M – 35.1M   | Apr 2021               | `FDE9D1FF--35008000-35071999` | 64,000           | 3.5 GB          | ~56.7 KB      |
| 38M – 38.1M   | ~Jan 2022              | `FDBBEBFF--38016000-38079999` | 64,000           | 8.6 GB          | ~144.7 KB     |
| 41M – 41.1M   | ~Jun 2022              | `FD8EFFFF--40960000-41023999` | 64,000           | 12.8 GB         | ~199.7 KB     |
| 44M – 44.1M   | ~Dec 2022              | `FD5F25FF--44096000-44159999` | 64,000           | 11.5 GB         | ~188.4 KB     |
| 46M – 46.1M   | Apr 2023               | `FD41D9FF--46016000-46079999` | 64,000           | 13.3 GB         | ~217.2 KB     |
| **50.46M**    | **Feb 2024 ← Soroban** | `FCFE77FF--50432000-50495999` | 64,000           | 9.7 GB          | ~159.2 KB     |
| 51M – 51.1M   | ~Mar 2024              | `FCF6A7FF--50944000-51007999` | 64,000           | 11.3 GB         | ~185 KB       |
| 62M – present | Apr 2026               | `FC4DB5FF--62016000-62079999` | 23,675 (partial) | 4.3 GB          | ~181 KB       |

### Key observations

- **Early ledgers (0–10M, 2015–2016):** nearly empty, 266–500 bytes each. Minimal transaction volume.
- **Middle growth (10M–31M, 2016–2020):** steady increase to ~30 KB. Network activity and DEX usage expand.
- **Rapid growth (31M–41M, 2020–2022):** files grow 6× from 31 KB to 200 KB. Correlates with AMM launch (Protocol 18, Nov 2021) and other activity spikes.
- **Pre-Soroban plateau (41M–50.4M, 2022–2024):** sizes stabilize at ~160–220 KB.
- **Soroban era (50.46M+, Feb 2024–present):** no dramatic size jump at activation — the network was already operating at high load. Files settle at ~180 KB per ledger.

---

## Estimated Total Size

### By ledger band

| Ledger Band            | Approx Dates            | ~Ledgers    | ~Avg Size | ~Band Total |
| ---------------------- | ----------------------- | ----------- | --------- | ----------- |
| 0 – 10M                | Sep 2015 – Nov 2016     | 10,000,000  | 400 B     | 4 GB        |
| 10M – 21M              | Nov 2016 – Nov 2018     | 11,000,000  | 5 KB      | 55 GB       |
| 21M – 31M              | Nov 2018 – mid-2020     | 10,000,000  | 20 KB     | 200 GB      |
| 31M – 35M              | mid-2020 – Apr 2021     | 4,000,000   | 44 KB     | 176 GB      |
| 35M – 38M              | Apr 2021 – Jan 2022     | 3,000,000   | 100 KB    | 300 GB      |
| 38M – 50.46M           | Jan 2022 – Feb 2024     | 12,460,000  | 190 KB    | 2,367 GB    |
| **Total pre-Soroban**  | **Sep 2015 – Feb 2024** | **~50.46M** |           | **~3.1 TB** |
| 50.46M – 62M           | Feb 2024 – Apr 2026     | 11,580,000  | 183 KB    | 2,120 GB    |
| **Total post-Soroban** | **Feb 2024 – present**  | **~11.6M**  |           | **~2.1 TB** |
| **Grand total**        |                         | **~62M**    |           | **~5.2 TB** |

> Note: Approximated from sampling ~12 partitions. Actual total likely falls in the **5–7 TB** range. Soroban accounts for roughly **40% of total storage** despite covering only ~19% of the ledger history, reflecting the sustained high-throughput era.

---

## Access

The bucket is public. No AWS credentials required:

```bash
aws s3 ls s3://aws-public-blockchain/v1.1/stellar/ledgers/pubnet/ --no-sign-request
```

Config file with schema metadata:

```bash
aws s3 cp s3://aws-public-blockchain/v1.1/stellar/ledgers/pubnet/.config.json - --no-sign-request
# {"networkPassphrase":"Public Global Stellar Network ; September 2015","version":"1.0","compression":"zstd","ledgersPerBatch":1,"batchesPerPartition":64000}
```
