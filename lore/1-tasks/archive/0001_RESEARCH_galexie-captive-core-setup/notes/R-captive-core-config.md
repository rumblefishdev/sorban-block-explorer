---
prefix: R
title: 'Captive Core configuration and networking'
status: developing
spawned_from: null
spawns: []
---

# Captive Core Configuration and Networking

Research into how Stellar Captive Core is configured for use with Galexie, including TOML templates, network passphrases, peer connection behaviour, and binary requirements.

## What is Captive Core?

Captive Core invokes the `stellar-core` binary as a **subprocess** to stream ledgers from the Stellar network. Non-validating tools like Galexie and Stellar RPC bundle an optimised "Captive" Core to serve their operational needs. It can stream a ledger range from the past or stream new ledgers whenever they are confirmed by the network.

## Binary Requirements

- The `stellar-core` binary must be **installed before** Captive Core can be used.
- Two installation options:
  - Build from source: follow the [Installation Guide](https://developers.stellar.org/docs/validators/admin-guide/installation#installing-from-source)
  - Install via package manager: [Package-based Installation](https://developers.stellar.org/docs/validators/admin-guide/installation#package-based-installation)
- Verify installation: `./stellar-core version`
- Default binary path assumed by tooling: `/usr/bin/stellar-core` (also seen as `/usr/local/bin/stellar-core`)
- The `STELLAR_CORE_BINARY_PATH` environment variable (or config key `STELLAR_CORE_BINARY_PATH`) points to the binary.
- In containers (Docker/Kubernetes), the binary is bundled inside the `stellar/stellar-galexie` image — no separate install needed.

**BucketListDB note:** As of August 2024, Stellar Core officially transitioned to BucketListDB as its primary database backend (source: `developers-stellar-org__docs-validators-admin-guide-prerequisites.md`). For captive core configs, use `CAPTIVE_CORE_STORAGE_PATH` (Stellar RPC / Horizon-level parameter) to set the working directory; `BUCKET_DIR_PATH` in captive core configs may cause errors (source: `github-stellar-core-example-cfg.md`).

**Hardware minimums for Galexie:**

- RAM: 16 GB
- CPU: 4 vCPUs
- Persistent Disk: 100 GB with at least 5K IOPS

## Network Passphrases

These are the canonical passphrases used for signing and connecting to Stellar networks:

| Network                  | Passphrase                                       |
| ------------------------ | ------------------------------------------------ |
| Mainnet (Public Network) | `Public Global Stellar Network ; September 2015` |
| Testnet                  | `Test SDF Network ; September 2015`              |
| Future testnet           | `Test SDF Future Network ; October 2022`         |

If an incorrect passphrase is used when signing transactions, they will fail with a bad authentication error. The passphrase also derives the root account — changing it changes the root account.

## Captive Core TOML / CFG Configuration

Captive Core requires a **minimum** of:

1. The **network passphrase** (`NETWORK_PASSPHRASE`)
2. The path to **history archives** (`HISTORY_ARCHIVE_URLS`)

### Testnet captive-core.cfg template

This is the reference config from `go-stellar-sdk`:

```ini
NETWORK_PASSPHRASE="Test SDF Network ; September 2015"
UNSAFE_QUORUM=true
FAILURE_SAFETY=1

[[HOME_DOMAINS]]
HOME_DOMAIN="testnet.stellar.org"
QUALITY="HIGH"

[[VALIDATORS]]
NAME="sdf_testnet_1"
HOME_DOMAIN="testnet.stellar.org"
PUBLIC_KEY="GDKXE2OZMJIPOSLNA6N6F2BVCI3O777I2OOC4BV7VOYUEHYX7RTRYA7Y"
ADDRESS="core-testnet1.stellar.org"
HISTORY="curl -sf http://history.stellar.org/prd/core-testnet/core_testnet_001/{0} -o {1}"

[[VALIDATORS]]
NAME="sdf_testnet_2"
HOME_DOMAIN="testnet.stellar.org"
PUBLIC_KEY="GCUCJTIYXSOXKBSNFGNFWW5MUQ54HKRPGJUTQFJ5RQXZXNOLNXYDHRAP"
ADDRESS="core-testnet2.stellar.org"
HISTORY="curl -sf http://history.stellar.org/prd/core-testnet/core_testnet_002/{0} -o {1}"

[[VALIDATORS]]
NAME="sdf_testnet_3"
HOME_DOMAIN="testnet.stellar.org"
PUBLIC_KEY="GC2V2EFSXN6SQTWVYA5EPJPBWWIMSD2XQNKUOHGEKB535AQE2I6IXV2Z"
ADDRESS="core-testnet3.stellar.org"
HISTORY="curl -sf http://history.stellar.org/prd/core-testnet/core_testnet_003/{0} -o {1}"
```

Source: [go-stellar-sdk captive-core-testnet.cfg](https://github.com/stellar/go-stellar-sdk/blob/main/ingest/ledgerbackend/configs/captive-core-testnet.cfg)

### Mainnet (Pubnet) captive-core.cfg highlights

```ini
NETWORK_PASSPHRASE="Public Global Stellar Network ; September 2015"
FAILURE_SAFETY=1
HTTP_PORT=11626
PEER_PORT=11725
```

Full file has **21 validators** across 7 home domains (PublicNode, LOBSTR, Franklin Templeton, SatoshiPay, Creit Tech, SDF, Blockdaemon — 3 validators each).

**WARNING:** The default pubnet config from go-stellar-sdk includes the note: "Do not use this config in production. Quorum sets should be carefully selected manually."

Source: [go-stellar-sdk captive-core-pubnet.cfg](https://github.com/stellar/go-stellar-sdk/blob/main/ingest/ledgerbackend/configs/captive-core-pubnet.cfg)

### Galexie TOML configuration (galexie-config.toml)

Galexie uses a separate top-level TOML that wraps stellar core config. The `network` shorthand key auto-expands to the correct passphrase and history archives:

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

For explicit configuration (without the `network` shorthand):

```toml
[stellar_core_config]
  network_passphrase = "Test SDF Network ; September 2015"
  history_archive_urls = ["https://history.stellar.org/prd/core-testnet/core_testnet_001"]
  stellar_core_binary_path = "/usr/bin/stellar-core"
  captive_core_toml_path = "/config/stellar-core.cfg"
```

> **Note**: The correct key name is `captive_core_toml_path` (confirmed in `config/config.example.toml`). Do not confuse with `CAPTIVE_CORE_CONFIG_PATH` which is a Stellar RPC environment variable, or `CAPTIVE_CORE_STORAGE_PATH` which controls on-disk bucket file location (set via environment variable, not in Galexie TOML).

For S3 instead of GCS:

```toml
[datastore_config]
type = "S3"

[datastore_config.params]
destination_bucket_path = "my-s3-bucket/ledgers/testnet"
```

### Other Stellar tools (NOT Galexie — included for reference only)

> **Warning**: The environment variables and Go SDK code below are for **Stellar RPC** and **Go SDK ingest library**, not for Galexie. Galexie uses its own TOML config format (see above). These are included only for cross-reference when comparing Captive Core configurations across Stellar tooling.

**Stellar-captive-core-api default environment variables (Stellar RPC, not Galexie):**

```bash
## /etc/default/stellar-captive-core-api — THIS IS STELLAR RPC, NOT GALEXIE
STELLAR_CORE_BINARY_PATH=/usr/bin/stellar-core
STELLAR_CORE_CONFIG_PATH=/etc/stellar/stellar-core.cfg
NETWORK_PASSPHRASE="Test SDF Network ; September 2015"
HISTORY_ARCHIVE_URLS="https://history.stellar.org/prd/core-testnet/core_testnet_001"
CAPTIVE_CORE_PORT=8000
CAPTIVE_CORE_LOG_LEVEL=info
```

**Go SDK: constructing CaptiveCoreToml programmatically (for custom Go tools, not Galexie):**

```go
captiveCoreToml, err := ledgerbackend.NewCaptiveCoreTomlFromData(
    ledgerbackend.PubnetDefaultConfig,
    ledgerbackend.CaptiveCoreTomlParams{
        NetworkPassphrase:  network.PublicNetworkPassphrase,
        HistoryArchiveURLs: network.PublicNetworkhistoryArchiveURLs,
    },
)

config := ledgerbackend.CaptiveCoreConfig{
    BinaryPath:         "/usr/local/bin/stellar-core",
    NetworkPassphrase:  network.PublicNetworkPassphrase,
    HistoryArchiveURLs: network.PublicNetworkhistoryArchiveURLs,
    Toml:               captiveCoreToml,
}

captiveStellarCoreBackend, err := ledgerbackend.NewCaptive(config)
```

## Peer Connections and Reconnection

> **Source**: Peer settings and defaults from `github-stellar-core-example-cfg.md`. BoundedRange/UnboundedRange from `pkgdev-ledgerbackend.md`.

Captive Core relies on standard `stellar-core` overlay networking. Key peer-related settings:

| Setting                           | Default | Description                                                   |
| --------------------------------- | ------- | ------------------------------------------------------------- |
| `PEER_PORT`                       | 11625   | Port where other stellar-core instances connect               |
| `TARGET_PEER_CONNECTIONS`         | 8       | Outbound connections target; server connects until this count |
| `MAX_ADDITIONAL_PEER_CONNECTIONS` | -1      | When -1, uses `TARGET_PEER_CONNECTIONS × 8` for inbound       |
| `MAX_PENDING_CONNECTIONS`         | 500     | Non-authenticated pending connections limit                   |
| `PEER_AUTHENTICATION_TIMEOUT`     | 2s      | Unauthenticated peers dropped after this                      |
| `PEER_TIMEOUT`                    | 30s     | Authenticated peers with no activity are dropped              |
| `PEER_STRAGGLER_TIMEOUT`          | 120s    | Peers that don't drain outgoing queues are dropped            |

### Reconnection behaviour

- `KNOWN_PEERS` — IP:port strings added to the peer database; the node attempts connections to these when below `TARGET_PEER_CONNECTIONS`.
- `PREFERRED_PEERS` — peers given connection priority.
- `PREFERRED_PEER_KEYS` — validator keys from the quorum set; prioritised to aid consensus reachability.
- `PREFERRED_PEERS_ONLY` — when `true`, connects **only** to preferred peers.
- Core continuously reconnects to `KNOWN_PEERS` and `PREFERRED_PEERS` to maintain the `TARGET_PEER_CONNECTIONS` count. No explicit reconnect interval is exposed; the overlay manager handles this internally.

### Firewall requirements

**For a full validator node** (NOT Galexie):

- Inbound: allow all IPs on TCP `PEER_PORT` (default 11625).
- Outbound: allow connections to other peers on TCP `PEER_PORT`.

**For Galexie with Captive Core** (our use case):

- Inbound: **none required** — Captive Core runs as an embedded subprocess, not as a standalone peer accepting inbound connections.
- Outbound (live streaming / UnboundedRange): TCP to Stellar network peers on **varied ports** (11625 default, 11725 used by SDF pubnet cfg, others possible) + HTTPS/443 to history archives. Egress rules should not be limited to a single port.
- Outbound (backfill / BoundedRange): HTTPS/443 to history archives only (e.g. `history.stellar.org`).
- Outbound (both modes): HTTPS/443 to cloud storage (S3 or GCS) for writing exported ledger data.

### Captive Core-specific peer notes — BoundedRange vs UnboundedRange

Captive Core has **two distinct networking modes** depending on the ledger range type:

| Mode                       | Range type               | Networking                                                                                                                                      | Use case            |
| -------------------------- | ------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------- | ------------------- |
| **BoundedRange** (catchup) | `--start X --end Y`      | History archives only (HTTPS) — no peer connections                                                                                             | Historical backfill |
| **UnboundedRange** (live)  | `--start X` (no `--end`) | First catches up via archives, **then connects to Stellar network peers** (TCP on varied ports: 11625, 11725, others) for live ledger streaming | Live ingestion      |

Source: [ledgerbackend package docs](https://pkg.go.dev/github.com/stellar/go/ingest/ledgerbackend) — `PrepareRange` documentation states: "For UnboundedRange it will first catchup to starting ledger and then run it normally (including connecting to the Stellar network)."

**Key implication for ECS networking**: The live ingestion Galexie task requires NAT Gateway for outbound TCP to Stellar peers (on varied ports: 11625, 11725, others). The backfill-only task technically only needs HTTPS outbound to history archives, but using the same NAT Gateway setup for both simplifies infrastructure.

**Gotcha**: In UnboundedRange mode, if `GetLedger` is not called frequently (every ~5 seconds), Captive Core can go out of sync with the network because the communication pipe has no buffering.

### Peer connection failure and reconnection (live mode)

> **Source**: Reconnection via `OverlayManagerImpl::tick()` every 3s confirmed in `github-stellar-core-example-cfg.md`. GetLedger blocking behavior from `pkgdev-ledgerbackend.md`. Failure cascade is synthesis from these two sources.

When Captive Core is in UnboundedRange (live streaming) mode and loses peer connectivity:

1. **Reconnection is handled by stellar-core's overlay manager**, not by Galexie. Galexie does not implement any reconnection logic — it delegates entirely to Captive Core.
2. **stellar-core continuously attempts to reconnect** to `KNOWN_PEERS` and `PREFERRED_PEERS` to maintain the `TARGET_PEER_CONNECTIONS` count (default: 8). The overlay manager has no configurable reconnect interval; it manages this internally.
3. **If reconnection fails and the peer count stays at zero**, stellar-core cannot produce new ledgers. Galexie's `GetLedger` call will block waiting for the next ledger. If this exceeds the Captive Core internal timeout, the subprocess may exit with an error.
4. **If the stellar-core subprocess exits**, Galexie will terminate with an error. Galexie does **not** automatically restart Captive Core — this must be handled by the container orchestrator (ECS task restart policy, Kubernetes restartPolicy, etc.).
5. **On restart**, Galexie's `append` mode resumes from the last exported ledger in the datastore (via `findResumeLedger`), so no data is lost. The gap between the last exported ledger and the current network tip will be filled as Captive Core catches up.

**ECS implication**: Configure the ECS service with `desiredCount: 1` and let the service scheduler restart failed tasks automatically. The checkpoint-aware restart mechanism ensures no duplicate or missing ledgers.

The pubnet cfg sets `PEER_PORT=11725` (non-default) and `HTTP_PORT=11626`.

## Common Pitfalls and Troubleshooting

> **Source**: Error messages confirmed in `github-stellar-core-example-cfg.md` and the [Stellar RPC External Datastore Guide](https://gist.github.com/tmosleyIII/fcbc07f1dc936a904a4f4a0781ad8896).

| Error                                                             | Cause                             | Fix                                                                            |
| ----------------------------------------------------------------- | --------------------------------- | ------------------------------------------------------------------------------ |
| `setting BUCKET_DIR_PATH is disallowed for Captive Core`          | Legacy config key used            | Remove `BUCKET_DIR_PATH`; use `CAPTIVE_CORE_STORAGE_PATH`                      |
| `Fee stat analysis window cannot exceed history retention window` | Fee retention > history retention | Set fee stats retention windows ≤ `HISTORY_RETENTION_WINDOW`                   |
| `history-retention-window must be positive`                       | Value set to 0                    | Set `HISTORY_RETENTION_WINDOW = 1` minimum                                     |
| Bad auth errors on transactions                                   | Wrong network passphrase          | Verify passphrase matches network exactly, including spaces and capitalisation |

## History Archive URLs

### Testnet

| Node          | URL                                                            |
| ------------- | -------------------------------------------------------------- |
| SDF testnet 1 | `http://history.stellar.org/prd/core-testnet/core_testnet_001` |
| SDF testnet 2 | `http://history.stellar.org/prd/core-testnet/core_testnet_002` |
| SDF testnet 3 | `http://history.stellar.org/prd/core-testnet/core_testnet_003` |

### Mainnet (Pubnet)

When using `network = "pubnet"` in Galexie's config.toml, the Go SDK auto-resolves SDF's public archive URLs. For explicit configuration or custom captive-core.cfg, the primary SDF mainnet archive URLs are:

| Node       | URL (as used in pubnet cfg)                              |
| ---------- | -------------------------------------------------------- |
| SDF live 1 | `http://history.stellar.org/prd/core-live/core_live_001` |
| SDF live 2 | `http://history.stellar.org/prd/core-live/core_live_002` |
| SDF live 3 | `http://history.stellar.org/prd/core-live/core_live_003` |

> **Note**: The SDF pubnet captive-core.cfg uses `http://` (not `https://`) for SDF history archive URLs. The testnet cfg also uses `http://`. Third-party validators (PublicNode, LOBSTR, etc.) use `https://`. Both protocols work — `history.stellar.org` supports HTTPS. When configuring `history_archive_urls` in Galexie's TOML, either protocol is valid.

Additional third-party archives (from the pubnet captive-core.cfg validators):

| Operator           | URL(s)                                                                                                                                                                                            |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| PublicNode         | `https://bootes-history.publicnode.org`, `https://lyra-history.publicnode.org`, `https://hercules-history.publicnode.org`                                                                         |
| LOBSTR             | `https://archive.v1.stellar.lobstr.co`, `https://archive.v2.stellar.lobstr.co`, `https://archive.v5.stellar.lobstr.co`                                                                            |
| Franklin Templeton | `https://stellar-history-usc.franklintempleton.com/azuscshf401`, `https://stellar-history-ins.franklintempleton.com/azinsshf401`, `https://stellar-history-usw.franklintempleton.com/azuswshf401` |
| SatoshiPay         | `https://stellar-history-de-fra.satoshipay.io`, `https://stellar-history-sg-sin.satoshipay.io`, `https://stellar-history-us-iowa.satoshipay.io`                                                   |
| Creit Tech         | `https://gamma-history.validator.stellar.creit.tech`, `https://alpha-history.validator.stellar.creit.tech`, `https://beta-history.validator.stellar.creit.tech`                                   |
| Blockdaemon        | `https://stellar-full-history1.bdnodes.net`, `https://stellar-full-history2.bdnodes.net`, `https://stellar-full-history3.bdnodes.net`                                                             |

> **Note**: All history archive URLs point to the **public internet** (not AWS services). Even for backfill-only tasks (BoundedRange), outbound HTTPS/443 to these URLs is required, which means NAT Gateway or public subnet connectivity is needed in all deployment modes.

## Sources

- [Captive Core | Stellar Docs (Ingest SDK developer guide)](https://developers.stellar.org/docs/data/indexers/build-your-own/ingest-sdk/developer_guide/ledgerbackends/captivecore)
- [Export to GCS | Stellar Docs (Galexie example)](https://developers.stellar.org/docs/data/indexers/build-your-own/galexie/examples/gcs-export)
- [Configuring Stellar RPC | Stellar Docs](https://developers.stellar.org/docs/data/apis/rpc/admin-guide/configuring)
- [Galexie Prerequisites | Stellar Docs](https://developers.stellar.org/docs/data/indexers/build-your-own/galexie/admin_guide/prerequisites)
- [captive-core-testnet.cfg | go-stellar-sdk GitHub](https://github.com/stellar/go-stellar-sdk/blob/main/ingest/ledgerbackend/configs/captive-core-testnet.cfg)
- [captive-core-pubnet.cfg | go-stellar-sdk GitHub](https://github.com/stellar/go-stellar-sdk/blob/main/ingest/ledgerbackend/configs/captive-core-pubnet.cfg)
- [stellar-core_example.cfg | stellar/stellar-core GitHub](https://github.com/stellar/stellar-core/blob/master/docs/stellar-core_example.cfg)
- [stellar-captive-core-api.default | stellar/packages GitHub](https://github.com/stellar/packages/blob/master/stellar-captive-core-api/debian/stellar-captive-core-api.default)
- [Stellar RPC External Datastore Guide | GitHub Gist](https://gist.github.com/tmosleyIII/fcbc07f1dc936a904a4f4a0781ad8896)
- [Peer connection issue discussion | stellar/stellar-core GitHub](https://github.com/stellar/stellar-core/issues/1812)
- [ledgerbackend package | pkg.go.dev](https://pkg.go.dev/github.com/stellar/go/ingest/ledgerbackend)
