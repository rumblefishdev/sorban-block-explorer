---
url: 'https://github.com/stellar/go/blob/master/support/datastore/schema.go'
title: 'stellar/go support/datastore/schema.go'
fetched_date: 2026-03-25
task_id: '0001'
note: 'Repository stellar/go was archived Dec 16, 2025. Functionality migrated to go-stellar-sdk.'
---

# stellar/go: support/datastore/schema.go

**Verification result:** `GetObjectKeyFromSequenceNumber` - CONFIRMED. Hex-prefix key format uses `math.MaxUint32 - sequenceNumber` formatted as `%08X`, which produces an 8-character zero-padded uppercase hex string. This is a reverse-sorted prefix to ensure newer ledgers sort before older ones in object storage.

## Full Source

```go
package datastore

import (
	"fmt"
	"math"

	"github.com/stellar/go/support/compressxdr"
)

type DataStoreSchema struct {
	LedgersPerFile    uint32 `toml:"ledgers_per_file"`
	FilesPerPartition uint32 `toml:"files_per_partition"`
	FileExtension     string // Optional - for backward (zstd) compatibility only
}

func (ec DataStoreSchema) GetSequenceNumberStartBoundary(ledgerSeq uint32) uint32 {
	if ec.LedgersPerFile == 0 {
		return 0
	}
	return (ledgerSeq / ec.LedgersPerFile) * ec.LedgersPerFile
}

func (ec DataStoreSchema) GetSequenceNumberEndBoundary(ledgerSeq uint32) uint32 {
	return ec.GetSequenceNumberStartBoundary(ledgerSeq) + ec.LedgersPerFile - 1
}

func (ec DataStoreSchema) GetObjectKeyFromSequenceNumber(ledgerSeq uint32) string {
	var objectKey string

	if ec.FilesPerPartition > 1 {
		partitionSize  := ec.LedgersPerFile * ec.FilesPerPartition
		partitionStart := (ledgerSeq / partitionSize) * partitionSize
		partitionEnd   := partitionStart + partitionSize - 1

		objectKey = fmt.Sprintf("%08X--%d-%d/", math.MaxUint32-partitionStart, partitionStart, partitionEnd)
	}

	fileStart := ec.GetSequenceNumberStartBoundary(ledgerSeq)
	fileEnd   := ec.GetSequenceNumberEndBoundary(ledgerSeq)
	objectKey += fmt.Sprintf("%08X--%d", math.MaxUint32-fileStart, fileStart)

	// Multiple ledgers per file
	if fileStart != fileEnd {
		objectKey += fmt.Sprintf("-%d", fileEnd)
	}

	if ec.FileExtension == "" {
		ec.FileExtension = compressxdr.DefaultCompressor.Name()
	}

	objectKey += fmt.Sprintf(".xdr.%s", ec.FileExtension)

	return objectKey
}
```

## Key Format Analysis

### Hex-Prefix Pattern

The hex prefix uses **inverted sequence numbers**: `math.MaxUint32 - sequenceNumber`.

`math.MaxUint32` = `4294967295` = `0xFFFFFFFF`

This inversion means:

- Lower ledger sequence numbers → higher hex prefix
- Newer ledgers have lower hex prefixes (sorts first in reverse-ordered object stores)

Format string: `%08X` = 8-character zero-padded uppercase hexadecimal.

### Key Structure Examples

**Single-ledger-per-file, no partitioning** (`LedgersPerFile=1`, `FilesPerPartition=1`):

```
FFFFFFFE--1.xdr.zst       (ledger 1)
FFFFFFFD--2.xdr.zst       (ledger 2)
FFFF9C5F--25505.xdr.zst   (ledger 25505)
```

**Multi-ledger files with partitioning** (`LedgersPerFile=64`, `FilesPerPartition=10`):

```
FFFFFFC0--0-639/FFFFFFC0--0-63.xdr.zst
```

Pattern: `{partitionHex}--{partStart}-{partEnd}/{fileHex}--{fileStart}-{fileEnd}.xdr.{ext}`

### File Extension

`FileExtension` defaults to `compressxdr.DefaultCompressor.Name()` which is `"zst"` (standard Zstandard extension). The `DataStoreSchema.FileExtension` field is marked "Optional - for backward (zstd) compatibility only" - older files used `.zstd` extension, newer ones use `.zst`.

### Verification Against Notes

| Claim                                | Status    | Detail                                     |
| ------------------------------------ | --------- | ------------------------------------------ |
| Hex-prefix format is `%08X`          | CONFIRMED | 8-char zero-padded uppercase hex           |
| Uses `math.MaxUint32 - seq`          | CONFIRMED | Reverse-sorting trick                      |
| Extension appended as `.xdr.{ext}`   | CONFIRMED | e.g., `.xdr.zst`                           |
| Partition prefix uses `--` separator | CONFIRMED | `{hex}--{start}-{end}/`                    |
| File uses `--` separator             | CONFIRMED | `{hex}--{start}` or `{hex}--{start}-{end}` |
