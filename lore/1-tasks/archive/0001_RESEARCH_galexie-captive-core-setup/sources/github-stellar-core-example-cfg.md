---
url: 'https://github.com/stellar/stellar-core/blob/master/docs/stellar-core_example.cfg'
title: 'stellar-core example configuration file'
fetched_date: 2026-03-25
task_id: '0001'
overwritten: false
image_count: 0
---

# stellar-core Example Configuration

Source: `docs/stellar-core_example.cfg` in `stellar/stellar-core` on GitHub (master branch).

The intermediate fetch model processed this as a description of config parameters; values below are drawn from the raw file fetch.

---

## General Admin Settings

**LOG_FILE_PATH** — where stellar-core writes log output. Supports datetime patterns. Set to `""` to disable file logging.

**LOG_COLOR** — enables ANSI terminal colors in stdout (default: `false`).

**HISTOGRAM_WINDOW_SIZE** — time window in seconds for metric percentile calculations (default: `30`).

**BUCKET_DIR_PATH** — directory for the bucket list (default: `"buckets"`).

**DATABASE** — connection string for the database. Supports SQLite or PostgreSQL:

```
sqlite3://path/to/dbname.db
postgresql://dbname=stellar user=xxxx password=yyyy host=10.0.x.y
```

**ENTRY_CACHE_SIZE** — maximum cached LedgerEntry objects (default: `4096`).

**PREFETCH_BATCH_SIZE** — batch sizes for bulk load operations (default: `1000`).

**HTTP_PORT** — command interface port (default: `11626`). Set to `0` to disable.

**PUBLIC_HTTP_PORT** — when false, only localhost connections accepted. Never expose to the internet.

**HTTP_MAX_CLIENT** — concurrent HTTP clients (default: `128`).

**HTTP_QUERY_PORT** — query interface port (default: `0` = disabled).

**QUERY_THREAD_POOL_SIZE** — threads for query processing (default: `4`).

**QUERY_SNAPSHOT_LEDGERS** — historical ledger snapshots maintained for queries (default: `5`).

---

## Network Configuration

**NETWORK_PASSPHRASE** — identifies which Stellar network the instance joins.

- Mainnet: `"Public Global Stellar Network ; September 2015"`

---

## Overlay and Peer Settings

### Connection Management

**PEER_PORT** — port for inbound peer connections (default: `11625`).

**TARGET_PEER_CONNECTIONS** — desired outbound connections (default: `8`).

**MAX_ADDITIONAL_PEER_CONNECTIONS** — limits inbound peer connections. Setting to `-1` uses `TARGET_PEER_CONNECTIONS * 8` (default: `-1`).

**MAX_PENDING_CONNECTIONS** — caps non-authenticated connections (default: `500`).

### Peer Timeout Settings

**PEER_AUTHENTICATION_TIMEOUT** — drops peers failing authentication within this duration (default: `2` seconds).

**PEER_TIMEOUT** — drops authenticated peers with no activity (default: `30` seconds).

**PEER_STRAGGLER_TIMEOUT** — drops peers failing to drain outgoing queues (default: `120` seconds).

### Message Batching

**MAX_BATCH_WRITE_COUNT** — messages per peer transmission (default: `1024`).

**MAX_BATCH_WRITE_BYTES** — bytes per peer transmission (default: `1048576`).

### Transaction and Operation Flooding

**FLOOD_OP_RATE_PER_LEDGER** (default: `1.0`)

**FLOOD_TX_PERIOD_MS** (default: `200`)

**FLOOD_SOROBAN_RATE_PER_LEDGER** (default: `1.0`)

**FLOOD_SOROBAN_TX_PERIOD_MS** (default: `200`)

**FLOOD_ARB_TX_BASE_ALLOWANCE** — arbitrage path-payments per period (default: `5`). Set to `-1` to disable traffic damping.

**FLOOD_ARB_TX_DAMPING_FACTOR** (default: `0.8`).

### Pull-Mode Settings

**FLOOD_DEMAND_PERIOD_MS** (default: `200`)

**FLOOD_ADVERT_PERIOD_MS** (default: `100`)

**FLOOD_DEMAND_BACKOFF_DELAY_MS** (default: `500`)

### Peer Lists and Preferences

**KNOWN_PEERS** — bootstrap peer addresses for network discovery. Core connects to these on startup to find more peers.

**PREFERRED_PEERS** — IP:port addresses for persistent connection attempts. Core tries harder to connect and stay connected to these.

**PREFERRED_PEER_KEYS** — public keys treated as preferred peers.

**PREFERRED_PEERS_ONLY** — restricts connections exclusively to preferred peers when enabled (default: `false`).

**SURVEYOR_KEYS** — limits survey message relay to specified key origins.

---

## Reconnection Behavior

The overlay `tick()` function runs every 3 seconds and:

1. Connects to preferred peers first
2. Attempts regular outbound connections if slots available
3. Promotes inbound connections to outbound when space permits

When out-of-sync for longer than `OUT_OF_SYNC_RECONNECT_DELAY` (60 seconds), Core randomly disconnects a non-preferred peer hoping to pick a better one. Preferred peers are never affected by this logic.

Core continuously tries to maintain `TARGET_PEER_CONNECTIONS` outbound connections by periodically checking and initiating new connections when below the target.

---

## SCP and Consensus Settings

**NODE_SEED** — cryptographic seed for node identification in SCP. Generate using `stellar-core gen-seed`.

**NODE_IS_VALIDATOR** — enables SCP participation (default: `false`). Most instances operate as observers.

**NODE_HOME_DOMAIN** — validator's home domain. Required when `NODE_IS_VALIDATOR=true`.

**FAILURE_SAFETY** — tolerable validator failures (default: `-1` = automatic calculation as `(n-1)/3`).

**UNSAFE_QUORUM** — permits potentially unsafe quorum configurations (default: `false`).

---

## History and Catchup

**CATCHUP_COMPLETE** — replays all history when enabled (default: `false`).

**CATCHUP_RECENT** — ledger count for historical replay. Zero = minimal catchup using deltas (default: `0`).

**WORKER_THREADS** (default: `11`).

**COMPILATION_THREADS** (default: `6`).

**QUORUM_INTERSECTION_CHECKER** (default: `true`).

**MAX_CONCURRENT_SUBPROCESSES** (default: `16`).

**AUTOMATIC_SELF_CHECK_PERIOD** — self-check intervals in seconds (default: `10800`). Set to `0` to disable.

---

## Known Error Messages

### Captive Core

```
setting BUCKET_DIR_PATH is disallowed for Captive Core
```

This error is thrown when `BUCKET_DIR_PATH` is set in a config file used for Captive Core mode. The fix is to remove `BUCKET_DIR_PATH` from the captive-core cfg and use `CAPTIVE_CORE_STORAGE_PATH` in the parent config instead.

Source confirmation: Observed in practice (see `github-gist-stellar-rpc-external-datastore-guide.md`).

### Fee Statistics

```
Fee stat analysis window (50) cannot exceed history retention window (1)
```

Thrown when `SOROBAN_FEE_STATS_RETENTION_WINDOW` or `CLASSIC_FEE_STATS_RETENTION_WINDOW` exceeds `HISTORY_RETENTION_WINDOW`. Fix: ensure fee stat windows ≤ history retention window.

### History Retention

```
history-retention-window must be positive
```

Thrown when `HISTORY_RETENTION_WINDOW` is set to `0` or less. Fix: set to at least `1`.

---

## Validator Configuration

```toml
[[HOME_DOMAINS]]
HOME_DOMAIN="testnet.stellar.org"
QUALITY="HIGH"

[[VALIDATORS]]
NAME="sdftest1"
HOME_DOMAIN="testnet.stellar.org"
PUBLIC_KEY="GDKXE2OZMJIPOSLNA6N6F2BVCI3O777I2OOC4BV7VOYUEHYX7RTRYA7Y"
ADDRESS="core-testnet1.stellar.org"
HISTORY="curl -sf http://history.stellar.org/prd/core-testnet/core_testnet_001/{0} -o {1}"
```

---

## History Archives

```toml
[HISTORY.local]
get="cp /var/lib/stellar-core/history/vs/{0} {1}"
put="cp {0} /var/lib/stellar-core/history/vs/{1}"
mkdir="mkdir -p /var/lib/stellar-core/history/vs/{0}"
```

---

## Testing Parameters

**RUN_STANDALONE** — prevents peer connections for isolated testing (default: `false`).

**INVARIANT_CHECKS** — enables consistency checks including:

- `AccountSubEntriesCountIsValid`
- `BucketListIsConsistentWithDatabase`
- `CacheIsConsistentWithDatabase`
- `ConservationOfLumens`
- `LedgerEntryIsValid`
- `LiabilitiesMatchOffers`
- `EventsAreConsistentWithEntryDiffs`
- `ArchivedStateConsistency`

**ARTIFICIALLY_ACCELERATE_TIME_FOR_TESTING** — reduces ledger close time to 1 second (default: `false`). Incompatible with production networks.

**MANUAL_CLOSE** (default: `false`).

**ALLOW_LOCALHOST_FOR_TESTING** (default: `false`).

### Flow Control

**PEER_READING_CAPACITY** (default: `201`)

**PEER_FLOOD_READING_CAPACITY** (default: `200`)

**FLOW_CONTROL_SEND_MORE_BATCH_SIZE** (default: `40`)

**OUTBOUND_TX_QUEUE_BYTE_LIMIT** (default: `3145728`)

---

## Soroban Configuration

**ENABLE_SOROBAN_DIAGNOSTIC_EVENTS** (default: `false`)

**ENABLE_DIAGNOSTICS_FOR_TX_SUBMISSION** (default: `false`)

**EMIT_SOROBAN_TRANSACTION_META_EXT_V1** — emits Soroban resource fee breakdown (default: `false`).

**EMIT_LEDGER_CLOSE_META_EXT_V1** — emits dynamic Soroban write fees (default: `false`).

**EMIT_CLASSIC_EVENTS** (default: `false`)
