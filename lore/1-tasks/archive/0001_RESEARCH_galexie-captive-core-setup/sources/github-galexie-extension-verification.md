---
url: 'https://github.com/stellar/stellar-galexie/blob/main/internal/config.go'
title: 'File Extension Verification: .xdr.zst vs .xdr.zstd'
fetched_date: 2026-03-25
task_id: '0001'
---

# File Extension Verification: .xdr.zst vs .xdr.zstd

## VERDICT: CONFIRMED `.xdr.zst`

The actual file extension produced by galexie is **`.xdr.zst`**, not `.xdr.zstd`.

---

## Evidence Chain

### 1. galexie internal/config.go — compressionType constant

Source: https://github.com/stellar/stellar-galexie/blob/main/internal/config.go

```go
// user-configurable in the future.
const compressionType = "zstd"
```

This constant is named `"zstd"` but it is **not used as a file extension**. It is passed into the datastore config as the compression format identifier:

```go
config.DataStoreConfig.Compression = compressionType
```

This value selects the compression algorithm, not the file extension.

### 2. stellar/go support/datastore/schema.go — GetObjectKeyFromSequenceNumber

Source: https://github.com/stellar/go/blob/master/support/datastore/schema.go

```go
type DataStoreSchema struct {
    LedgersPerFile    uint32 `toml:"ledgers_per_file"`
    FilesPerPartition uint32 `toml:"files_per_partition"`
    FileExtension     string // Optional – for backward (zstd) compatibility only
}

func (ec DataStoreSchema) GetObjectKeyFromSequenceNumber(ledgerSeq uint32) string {
    // ... partition and file boundary logic ...

    if ec.FileExtension == "" {
        ec.FileExtension = compressxdr.DefaultCompressor.Name()
    }

    objectKey += fmt.Sprintf(".xdr.%s", ec.FileExtension)

    return objectKey
}
```

Key observations:

- The file extension is sourced from `compressxdr.DefaultCompressor.Name()` when not explicitly configured.
- The `FileExtension` field comment reads "Optional – for backward **(zstd)** compatibility only" — this means `.zstd` was the **old** extension. It is only kept for reading legacy files.

### 3. stellar/go support/compressxdr/compressor.go — DefaultCompressor.Name()

Source: https://github.com/stellar/go/blob/master/support/compressxdr/compressor.go

```go
var DefaultCompressor = &ZstdCompressor{}

type ZstdCompressor struct{}

// GetName returns the name of the compression algorithm.
func (z ZstdCompressor) Name() string {
    return "zst"
}
```

`DefaultCompressor.Name()` returns **`"zst"`**, which is substituted into `.xdr.%s` to produce **`.xdr.zst`**.

---

## Why the Confusion Exists

| Value                              | Where it appears                       | Meaning                                                                                      |
| ---------------------------------- | -------------------------------------- | -------------------------------------------------------------------------------------------- |
| `compressionType = "zstd"`         | galexie `internal/config.go`           | Selects the compression algorithm (Zstandard). Not a file extension.                         |
| `FileExtension = "zstd"`           | datastore schema `FileExtension` field | **Legacy** backward-compatibility override. Used only when explicitly set to read old files. |
| `DefaultCompressor.Name() = "zst"` | `compressxdr/compressor.go`            | **Current** default. Produces `.xdr.zst` on all new writes.                                  |

The monitoring docs showing `.xdr.zstd` in logs likely reflect either:

- Legacy data written before the extension was changed from `.zstd` to `.zst`, OR
- Outdated documentation that has not been updated to reflect the current `.zst` default.

---

## Conclusion

New files written by galexie use **`.xdr.zst`** as the file extension. The `.xdr.zstd` extension exists only for backward compatibility with older data and should not appear on freshly exported ledger files.
