---
url: 'https://github.com/stellar/stellar-galexie/blob/main/internal/config.go'
title: 'stellar-galexie internal/config.go'
fetched_date: 2026-03-25
task_id: '0001'
note: 'Requested path was config/config.go - actual file is internal/config.go (config/ directory only contains config.example.toml)'
---

# stellar-galexie: internal/config.go

**Verification result:** `const compressionType = "zstd"` - CONFIRMED. Found in `internal/config.go`.

**Additional finding:** `xdr.LedgerCloseMetaBatch` - CONFIRMED. This is the actual XDR type used (see `internal/ledger_meta_archive.go` below). The type comes from `github.com/stellar/go-stellar-sdk/xdr`.

## Key Constants and Types

```go
const (
	Pubnet     = "pubnet"
	Testnet    = "testnet"
	Futurenet  = "futurenet"
	UserAgent  = "galexie"
)

type Mode int

const (
	_          Mode = iota
	ScanFill   Mode = iota
	Append
	Replace
	DetectGaps
	LoadTest
)

const compressionType = "zstd"
```

## Mode Methods

Mode methods determine capabilities:

- `Mode.Export()` - true for ScanFill, Append, Replace, LoadTest
- `Mode.Resumable()` - true for Append only
- `Mode.Replace()` - true for Replace only
- `Mode.LoadTest()` - true for LoadTest only

Bounded (requires non-zero end): ScanFill, Replace, DetectGaps.

## RuntimeSettings Struct

```go
type RuntimeSettings struct {
	StartLedger           uint32
	EndLedger             uint32
	ConfigFilePath        string
	Mode                  Mode
	Ctx                   context.Context
	ReportWriter          io.Writer
	// Load test specific
	LoadTestMerge         bool
	LoadTestLedgersPath   string
	LoadTestCloseDuration time.Duration
}
```

## StellarCoreConfig Struct

```go
type StellarCoreConfig struct {
	NetworkName        string
	NetworkPassphrase  string
	HistoryArchiveURLs []string
	StellarCoreBinaryPath string
	CaptiveCoreTomlPath   string
}
```

## Config Struct

```go
type Config struct {
	DataStoreConfig          datastore.DataStoreConfig
	StellarCoreConfig        StellarCoreConfig
	StartLedger              uint32
	EndLedger                uint32
	AdminPort                int
	Mode                     Mode
	CoreVersion              string
	SerializedCaptiveCoreToml []byte
	CoreBuildVersionFn        func(ctx context.Context, coreBin string) (string, error)
	// Load test specific
	LoadTestMerge         bool
	LoadTestLedgersPath   string
	LoadTestCloseDuration time.Duration
}
```

## Primary Functions

- `NewConfig(runtimeSettings RuntimeSettings, overrides interface{}) (*Config, error)` - initializes from TOML file, merges network overlays
- `ValidateLedgerRange(archive historyarchive.ArchiveInterface) error` - validates start > 1, end constraints, range vs network current ledger
- `GenerateHistoryArchive(ctx context.Context, log *log.Entry) (historyarchive.ArchiveInterface, error)`
- `GenerateCaptiveCoreConfig(coreBinFromPath string) (ledgerbackend.CaptiveCoreConfig, error)`
- `adjustLedgerRange()` - aligns start/end to LedgersPerFile boundaries for schema consistency
- `countLoadTestLedgers(path string) (uint32, error)` - reads zstd-compressed XDR stream of `LedgerCloseMeta` entries

## Compression Details

The `compressionType = "zstd"` constant is used internally. The `countLoadTestLedgers` function creates an `xdr.NewZstdStream()` and reads individual `xdr.LedgerCloseMeta` objects sequentially until EOF. The current default compressor (as checked in `app.go`) is `compressxdr.DefaultCompressor.Name()` which evaluates to `"zst"` (the standard file extension), while `compressionType = "zstd"` is the algorithm name string used in metadata.

## LedgerCloseMetaBatch Usage (from internal/ledger_meta_archive.go)

```go
package galexie

import (
	"github.com/stellar/go-stellar-sdk/support/compressxdr"
	"github.com/stellar/go-stellar-sdk/support/datastore"
	"github.com/stellar/go-stellar-sdk/xdr"
)

// LedgerMetaArchive represents a file with metadata and binary data.
type LedgerMetaArchive struct {
	ObjectKey string
	Data      xdr.LedgerCloseMetaBatch
	metaData  datastore.MetaData
}

// NewLedgerMetaArchiveFromXDR creates a new LedgerMetaArchive instance.
func NewLedgerMetaArchiveFromXDR(networkPassPhrase string, coreVersion string, key string, data xdr.LedgerCloseMetaBatch) (*LedgerMetaArchive, error) {
	startLedger, err := data.GetLedger(uint32(data.StartSequence))
	if err != nil {
		return &LedgerMetaArchive{}, err
	}
	endLedger, err := data.GetLedger(uint32(data.EndSequence))
	if err != nil {
		return &LedgerMetaArchive{}, err
	}

	return &LedgerMetaArchive{
		ObjectKey: key,
		Data:      data,
		metaData: datastore.MetaData{
			StartLedger:          startLedger.LedgerSequence(),
			EndLedger:            endLedger.LedgerSequence(),
			StartLedgerCloseTime: startLedger.LedgerCloseTime(),
			EndLedgerCloseTime:   endLedger.LedgerCloseTime(),
			NetworkPassPhrase:    networkPassPhrase,
			CompressionType:      compressxdr.DefaultCompressor.Name(),
			ProtocolVersion:      endLedger.ProtocolVersion(),
			CoreVersion:          coreVersion,
			Version:              version,
		},
	}, nil
}
```

**Key finding:** `xdr.LedgerCloseMetaBatch` is the actual XDR type (from `go-stellar-sdk`). The type name `LedgerCloseMetaBatch` is confirmed. It has `StartSequence` and `EndSequence` fields, plus a `GetLedger(seq uint32)` method.
