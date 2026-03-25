---
url: 'https://pkg.go.dev/github.com/stellar/go/ingest/ledgerbackend'
title: 'ledgerbackend package - Go Packages'
fetched_date: 2026-03-25
task_id: '0001'
---

# ledgerbackend package

## Overview

The `ledgerbackend` package provides interfaces and implementations for accessing Stellar ledger data. This is part of the `github.com/stellar/go` module.

> **Deprecated:** Use `github.com/stellar/go-stellar-sdk` instead.

## Core Interface

### LedgerBackend

```go
type LedgerBackend interface {
    // GetLatestLedgerSequence returns the sequence of the latest ledger available
    // in the backend.
    GetLatestLedgerSequence(ctx context.Context) (sequence uint32, err error)

    // GetLedger will block until the ledger is available.
    GetLedger(ctx context.Context, sequence uint32) (xdr.LedgerCloseMeta, error)

    // PrepareRange prepares the given range (including from and to) to be loaded.
    // Some backends (like captive stellar-core) need to initialize data to be
    // able to stream ledgers. Blocks until the first ledger is available.
    PrepareRange(ctx context.Context, ledgerRange Range) error

    // IsPrepared returns true if a given ledgerRange is prepared.
    IsPrepared(ctx context.Context, ledgerRange Range) (bool, error)

    Close() error
}
```

## CaptiveStellarCore

The primary ledger backend implementation that manages an internal Stellar-Core subprocess.

### Type Definition

```go
type CaptiveStellarCore struct {
    // contains filtered or unexported fields
}
```

**Purpose:** A ledger backend that starts an internal Stellar-Core subprocess responsible for streaming ledger data. Provides better decoupling than DatabaseBackend but requires extra initialization time.

**Requirements:** Stellar-Core v13.2.0+

### Creating an Instance

```go
func NewCaptive(config CaptiveCoreConfig) (*CaptiveStellarCore, error)
```

### Configuration

```go
type CaptiveCoreConfig struct {
    // BinaryPath is the file path to the Stellar Core binary
    BinaryPath string

    // NetworkPassphrase is the Stellar network passphrase used by captive core
    // when connecting to the Stellar network
    NetworkPassphrase string

    // HistoryArchiveURLs are a list of history archive urls
    HistoryArchiveURLs []string

    // UserAgent is the value of `User-Agent` header that will be sent
    // along http archive requests.
    UserAgent string

    Toml *CaptiveCoreToml

    // CheckpointFrequency is the number of ledgers between checkpoints
    // if unset, DefaultCheckpointFrequency will be used
    CheckpointFrequency uint32

    // Log is an (optional) custom logger which will capture any output
    // from the Stellar Core process.
    // If Log is omitted then all output will be printed to stdout.
    Log *log.Entry

    // Context is the (optional) context which controls the lifetime
    // of a CaptiveStellarCore instance. Once the context is done
    // the CaptiveStellarCore instance will not be able to stream ledgers
    // from Stellar Core or spawn new instances of Stellar Core.
    // If Context is omitted CaptiveStellarCore will default to using context.Background.
    Context context.Context

    // StoragePath is the (optional) base path passed along to Core's
    // BUCKET_DIR_PATH which specifies where various bucket data should be stored.
    // We always append /captive-core to this directory, since we clean
    // it up entirely on shutdown.
    StoragePath string

    // CoreProtocolVersionFn is a function that returns the protocol version
    // of the stellar-core binary.
    CoreProtocolVersionFn CoreProtocolVersionFunc

    // CoreBuildVersionFn is a function that returns the build version
    // of the stellar-core binary.
    CoreBuildVersionFn CoreBuildVersionFunc
}
```

### Key Methods

#### PrepareRange

```go
func (c *CaptiveStellarCore) PrepareRange(ctx context.Context, ledgerRange Range) error
```

Prepares the given range (including from and to) to be loaded. Captive stellar-core backend needs to initialize Stellar-Core state to be able to stream ledgers.

**Behavior differs based on ledgerRange type:**

- **BoundedRange:** Stellar-Core starts in catchup mode to replay ledgers in memory. This is very fast but requires Stellar-Core to keep ledger state in RAM (approximately 3GB as of August 2020). Currently requires full-trust on history archive.

- **UnboundedRange:** First catches up to the starting ledger, then runs in normal mode (including connecting to the Stellar network). Requires the `configAppendPath` to be provided because a quorum set needs to be selected. All ledger entries must be stored on disk instead of RAM.

#### GetLedger

```go
func (c *CaptiveStellarCore) GetLedger(ctx context.Context, sequence uint32) (xdr.LedgerCloseMeta, error)
```

Retrieves the `LedgerCloseMeta` for a specific ledger sequence.

**Behavior:**

- Blocks until the ledger is available in the backend
- Returns data as `xdr.LedgerCloseMeta` - the XDR-encoded ledger metadata
- Call `PrepareRange` first to initialize Stellar-Core
- Requesting a ledger on a non-prepared backend returns an error
- Ledger sequences should be requested in non-decreasing order; requesting a sequence less than the last requested sequence returns an error
- Requesting a ledger far in the future will block execution for a long time
- For BoundedRange: After getting the last ledger in a range, this method also calls `Close()`

#### GetLatestLedgerSequence

```go
func (c *CaptiveStellarCore) GetLatestLedgerSequence(ctx context.Context) (uint32, error)
```

Returns the sequence of the latest ledger available in the backend.

**Note:** For UnboundedRange, the returned sequence number is not necessarily the latest sequence closed by the network. It's always the last value available in the backend.

#### IsPrepared

```go
func (c *CaptiveStellarCore) IsPrepared(ctx context.Context, ledgerRange Range) (bool, error)
```

Returns true if a given ledgerRange is prepared.

#### GetCoreVersion

```go
func (c *CaptiveStellarCore) GetCoreVersion() string
```

Returns the version of the Stellar Core binary.

#### Close

```go
func (c *CaptiveStellarCore) Close() error
```

Closes the Stellar-Core process, streaming sessions, and removes all temporary files.

**Important notes:**

- Once closed, the instance cannot be reused; all subsequent calls fail
- Thread-safe and can be called from another goroutine
- Creates a temporary folder for bucket files and temporary files; this folder is cleaned up on Close

### Operational Constraints

**Thread Safety:** Except for the `Close` function, CaptiveStellarCore is not thread-safe and should not be accessed by multiple goroutines.

**UnboundedRange Gotchas:**

- `PrepareRange` takes more time because ledger entries must be stored on disk instead of RAM
- If `GetLedger` is not called frequently (every 5 seconds on average), the Stellar-Core process can go out of sync with the network
- This happens because there is no buffering of the communication pipe and CaptiveStellarCore has a very small internal buffer
- Stellar-Core will not close the new ledger if it's not read

**Networking (UnboundedRange only):**

- CaptiveStellarCore connects to the Stellar network for UnboundedRange preparations
- A quorum set must be configured for network validation
- The `configAppendPath` parameter is required
- See `AddExamplePubnetValidators()` for example validator configuration

## Data Format

`GetLedger` returns `xdr.LedgerCloseMeta` - the XDR-encoded ledger metadata format used throughout the Stellar protocol.

## Range Types

```go
type Range struct {
    // contains filtered or unexported fields
}
```

### Creating Ranges

```go
// Bounded range with specific start and end
func BoundedRange(from uint32, to uint32) Range

// Single ledger
func SingleLedgerRange(ledger uint32) Range

// Unbounded starting from a ledger
func UnboundedRange(from uint32) Range
```

### Range Methods

```go
func (r Range) Bounded() bool              // Returns true if range has an upper bound
func (r Range) Contains(other Range) bool  // Checks if range contains another
func (r Range) From() uint32               // Starting ledger sequence
func (r Range) To() uint32                 // Ending ledger sequence (0 if unbounded)
func (r Range) String() string             // String representation
func (r Range) MarshalJSON() ([]byte, error)
func (r *Range) UnmarshalJSON(b []byte) error
```

## Error Types

### ErrCannotStartFromGenesis

```go
var ErrCannotStartFromGenesis = errors.New("CaptiveCore is unable to start from ledger 1, start from ledger 2")
```

Returned when attempting to prepare a range from ledger 1.

### ErrCannotCatchupAheadLatestCheckpoint

```go
var ErrCannotCatchupAheadLatestCheckpoint = errors.New("CaptiveCore is unable to catchup ahead of latest checkpoint")
```

Returned when attempting to prepare a bounded range where the ending ledger is ahead of the latest history archive snapshot.

## CaptiveCoreToml Configuration

```go
type CaptiveCoreToml struct {
    // contains filtered or unexported fields
}
```

### Constructors

```go
func NewCaptiveCoreToml(params CaptiveCoreTomlParams) (*CaptiveCoreToml, error)
func NewCaptiveCoreTomlFromFile(configPath string, params CaptiveCoreTomlParams) (*CaptiveCoreToml, error)
func NewCaptiveCoreTomlFromData(data []byte, params CaptiveCoreTomlParams) (*CaptiveCoreToml, error)
```

### Parameters

```go
type CaptiveCoreTomlParams struct {
    NetworkPassphrase string
    HistoryArchiveURLs []string
    HTTPPort *uint
    PeerPort *uint
    LogPath *string
    Strict bool
    CoreBinaryPath string
    EnforceSorobanDiagnosticEvents bool
    EnforceSorobanTransactionMetaExtV1 bool
    EmitUnifiedEvents bool
    EmitUnifiedEventsBeforeProtocol22 bool
    HTTPQueryServerParams *HTTPQueryServerParams
    CoreBuildVersionFn CoreBuildVersionFunc
    CoreProtocolVersionFn CoreProtocolVersionFunc
    BackfillRestoreMeta *bool
    EmitVerboseMeta bool
}
```

### Methods

```go
func (c *CaptiveCoreToml) HistoryIsConfigured() bool
func (c *CaptiveCoreToml) QuorumSetIsConfigured() bool
func (c *CaptiveCoreToml) Marshal() ([]byte, error)
func (c *CaptiveCoreToml) CatchupToml() (*CaptiveCoreToml, error)
func (c *CaptiveCoreToml) AddExamplePubnetValidators()
```

## Alternative Backend Implementations

### RPCLedgerBackend

```go
type RPCLedgerBackend struct {
    // contains filtered or unexported fields
}

func NewRPCLedgerBackend(options RPCLedgerBackendOptions) *RPCLedgerBackend

type RPCLedgerBackendOptions struct {
    RPCServerURL string      // Required: URL of the Stellar RPC server
    BufferSize uint32        // Optional: ledger retrieval buffer size (default: 10)
    HttpClient *http.Client  // Optional: custom HTTP client
}
```

Fetches ledger data from a Stellar RPC server with built-in buffering.

### BufferedStorageBackend

```go
type BufferedStorageBackend struct {
    // contains filtered or unexported fields
}

func NewBufferedStorageBackend(
    config BufferedStorageBackendConfig,
    dataStore datastore.DataStore,
    schema datastore.DataStoreSchema,
) (*BufferedStorageBackend, error)

type BufferedStorageBackendConfig struct {
    BufferSize uint32
    NumWorkers uint32
    RetryLimit uint32
    RetryWait  time.Duration
}
```

Reads ledger data from a storage service (generated by ledgerExporter).

## Utility Functions

```go
func CoreBuildVersion(coreBinaryPath string) (string, error)
```

Executes the `stellar-core version` command and extracts the core version (format: `vX.Y.Z-*`).

```go
func CoreProtocolVersion(coreBinaryPath string) (uint, error)
```

Retrieves the ledger protocol version from the stellar-core binary.

## Metrics

```go
func WithMetrics(
    base LedgerBackend,
    registry *prometheus.Registry,
    namespace string,
) LedgerBackend
```

Decorates a LedgerBackend with Prometheus metrics.

## Package Variables

```go
var (
    PubnetDefaultConfig []byte  // Embedded captive-core-pubnet.cfg
    TestnetDefaultConfig []byte // Embedded captive-core-testnet.cfg
)
```

## Additional Types

### Validator

```go
type Validator struct {
    Name       string
    Quality    string
    HomeDomain string
    PublicKey  string
    Address    string
    History    string
}
```

Represents a `[[VALIDATORS]]` entry in the captive core TOML configuration.

### QuorumSet

```go
type QuorumSet struct {
    ThresholdPercent int
    Validators       []string
}
```

Represents a `[QUORUM_SET]` table in the captive core TOML configuration.

### History

```go
type History struct {
    Get   string
    Put   string
    Mkdir string
}
```

Represents a `[HISTORY]` table in the captive core TOML configuration.
