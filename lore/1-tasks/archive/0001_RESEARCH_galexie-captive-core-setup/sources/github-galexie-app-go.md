---
url: 'https://github.com/stellar/stellar-galexie/blob/main/internal/app.go'
title: 'stellar-galexie internal/app.go'
fetched_date: 2026-03-25
task_id: '0001'
note: 'Requested path was cmd/app.go - actual file is internal/app.go (cmd/ only contains main.go with CLI wiring)'
---

# stellar-galexie: internal/app.go

**Verification result:** `defaultNumWorkers = 32` - CONFIRMED. Found in `internal/app.go`, not `cmd/app.go` (that file does not exist).

## Constants

```go
const (
	adminServerReadTimeout     = 5 * time.Second
	adminServerShutdownTimeout = time.Second * 5
	// TODO: make this timeout configurable
	uploadShutdownTimeout = 10 * time.Second
	// We expect the queue size to rarely exceed 1 or 2 because
	// upload speeds are expected to be much faster than the rate at which
	// captive core emits ledgers. However, configuring a higher capacity
	// than our expectation is useful because if we observe a large queue
	// size in our metrics that is an indication that uploads to the
	// data store have degraded
	uploadQueueCapacity = 128
	nameSpace           = "galexie"
)
```

```go
const defaultTaskSize = uint32(100_000)

// TODO: make this configurable
const defaultNumWorkers = 32
```

## Usage of defaultNumWorkers

```go
func (a *App) runDetectGaps(ctx context.Context, reportWriter io.Writer) error {
	from := a.config.StartLedger
	to   := a.config.EndLedger

	sc, _ := scan.NewScanner(
		a.dataStore,
		a.config.DataStoreConfig.Schema,
		defaultNumWorkers,
		defaultTaskSize,
		logger,
	)
	// ...
}
```

`defaultNumWorkers = 32` is used exclusively for the `detect-gaps` sub-command's parallel scanner, not for the main export pipeline. The export pipeline uses an upload queue (capacity 128) with two goroutines (one uploader, one export manager).

## Full Source

```go
package galexie

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"os/exec"
	"os/signal"
	"runtime/debug"
	"sync"
	"syscall"
	"time"

	"github.com/pkg/errors"
	"github.com/prometheus/client_golang/prometheus"
	"github.com/prometheus/client_golang/prometheus/collectors"
	"github.com/prometheus/client_golang/prometheus/promhttp"

	"github.com/stellar/go-stellar-sdk/historyarchive"
	"github.com/stellar/go-stellar-sdk/ingest/ledgerbackend"
	"github.com/stellar/go-stellar-sdk/ingest/loadtest"
	"github.com/stellar/go-stellar-sdk/support/compressxdr"
	"github.com/stellar/go-stellar-sdk/support/datastore"
	supporthttp "github.com/stellar/go-stellar-sdk/support/http"
	"github.com/stellar/go-stellar-sdk/support/log"

	"github.com/stellar/stellar-galexie/internal/scan"
)

const (
	adminServerReadTimeout     = 5 * time.Second
	adminServerShutdownTimeout = time.Second * 5
	// TODO: make this timeout configurable
	uploadShutdownTimeout = 10 * time.Second
	uploadQueueCapacity   = 128
	nameSpace             = "galexie"
)

var (
	logger  = log.New().WithField("service", nameSpace)
	version = "develop"
)

func SetLogOutput(w io.Writer) { logger.SetOutput(w) }

func init() {
	if version == "develop" {
		if bi, ok := debug.ReadBuildInfo(); ok && bi.Main.Version != "" && bi.Main.Version != "(devel)" {
			version = bi.Main.Version
		}
	}
}

func Version() string {
	return version
}

func NewDataAlreadyExportedError(Start uint32, End uint32) *DataAlreadyExportedError {
	return &DataAlreadyExportedError{Start: Start, End: End}
}

type DataAlreadyExportedError struct {
	Start uint32
	End   uint32
}

func (m DataAlreadyExportedError) Error() string {
	return fmt.Sprintf("For export ledger range start=%d, end=%d, the remote storage has all the data, there is no need to continue export", m.Start, m.End)
}

func NewInvalidDataStoreError(LedgerSequence uint32, LedgersPerFile uint32) *InvalidDataStoreError {
	return &InvalidDataStoreError{LedgerSequence: LedgerSequence, LedgersPerFile: LedgersPerFile}
}

type InvalidDataStoreError struct {
	LedgerSequence uint32
	LedgersPerFile uint32
}

func (m InvalidDataStoreError) Error() string {
	return fmt.Sprintf("The remote data store has inconsistent data, "+
		"a resumable starting ledger of %v was identified, "+
		"but that is not aligned to expected ledgers-per-file of %v. use 'scan-and-fill' sub-command to bypass",
		m.LedgerSequence, m.LedgersPerFile)
}

type DetectGapsOutput struct {
	ScanFrom uint32      `json:"scan_from"`
	ScanTo   uint32      `json:"scan_to"`
	Duration string      `json:"duration,omitempty"`
	Report   scan.Report `json:"report"`
}

type App struct {
	config        *Config
	ledgerBackend ledgerbackend.LedgerBackend
	dataStore     datastore.DataStore
	exportManager *ExportManager
	uploader      Uploader
	adminServer   *http.Server
}

func NewApp() *App {
	logger.SetLevel(log.DebugLevel)
	return &App{}
}

func (a *App) init(ctx context.Context, runtimeSettings RuntimeSettings) error {
	var err error
	var archive historyarchive.ArchiveInterface

	logger.Infof("Starting Galexie with version %s", version)

	if a.config, err = NewConfig(runtimeSettings, nil); err != nil {
		return errors.Wrap(err, "Could not load configuration")
	}
	if archive, err = a.config.GenerateHistoryArchive(ctx, logger); err != nil {
		return err
	}
	if err = a.config.ValidateLedgerRange(archive); err != nil {
		return err
	}
	if err = a.initDataStore(ctx); err != nil {
		return err
	}
	if a.config.Mode.Export() {
		if err = a.initExportPipeline(ctx); err != nil {
			return err
		}
	}
	return nil
}

func (a *App) initDataStore(ctx context.Context) error {
	var err error
	if a.dataStore, err = datastore.NewDataStore(ctx, a.config.DataStoreConfig); err != nil {
		return fmt.Errorf("could not connect to destination data store %w", err)
	}

	if a.config.Mode.LoadTest() {
		files, listErr := a.dataStore.ListFilePaths(ctx, datastore.ListFileOptions{Limit: 5})
		if listErr != nil {
			return fmt.Errorf("could not list datastore files for load test validation: %w", listErr)
		}
		if len(files) > 0 {
			return fmt.Errorf("load test mode requires an empty datastore, however, found existing files")
		}
	}

	if err = validateExistingFileExtension(ctx, a.dataStore); err != nil {
		return err
	}

	schema, err := datastore.LoadSchema(ctx, a.dataStore, a.config.DataStoreConfig)
	if err != nil {
		return fmt.Errorf("failed to retrieve datastore schema: %w", err)
	}
	a.config.DataStoreConfig.Schema = schema
	return nil
}

func (a *App) initExportPipeline(ctx context.Context) error {
	registry := prometheus.NewRegistry()
	registry.MustRegister(
		collectors.NewProcessCollector(collectors.ProcessCollectorOpts{Namespace: nameSpace}),
		collectors.NewGoCollector(),
	)

	a.config.adjustLedgerRange()

	manifest, created, err := datastore.PublishConfig(ctx, a.dataStore, a.config.DataStoreConfig)
	if err != nil {
		return fmt.Errorf("could not configure datastore %w", err)
	}
	if created {
		logger.WithField("manifest", manifest).Infof("Successfully created datastore config manifest.")
	} else {
		logger.WithField("manifest", manifest).Infof("Datastore config manifest already exists.")
	}

	if a.config.Mode.Resumable() {
		if err = a.applyResumability(ctx); err != nil {
			return err
		}
	}

	logger.Infof("Final computed ledger range for backend retrieval and export, start=%d, end=%d",
		a.config.StartLedger, a.config.EndLedger)

	if a.ledgerBackend, err = newLedgerBackend(a.config, registry); err != nil {
		return err
	}

	queue := NewUploadQueue(uploadQueueCapacity, registry)
	if a.exportManager, err = NewExportManager(a.config.DataStoreConfig.Schema,
		a.ledgerBackend, queue, registry,
		a.config.StellarCoreConfig.NetworkPassphrase,
		a.config.CoreVersion); err != nil {
		return err
	}
	a.uploader = NewUploader(a.dataStore, queue, registry, a.config.Mode.Replace())

	if a.config.AdminPort != 0 {
		a.adminServer = newAdminServer(a.config.AdminPort, registry)
	}
	return nil
}

func validateExistingFileExtension(ctx context.Context, ds datastore.DataStore) error {
	fileExt, err := datastore.GetLedgerFileExtension(ctx, ds)
	if err != nil {
		if errors.Is(err, datastore.ErrNoLedgerFiles) {
			log.Infof("no existing ledger files found in data store")
			return nil
		}
		return fmt.Errorf("unable to determine ledger file extension from data store: %w", err)
	}

	if fileExt != compressxdr.DefaultCompressor.Name() {
		return fmt.Errorf("detected older incompatible ledger files in the data store (extension %q). "+
			"Galexie v23.0+ requires starting with an empty datastore", fileExt)
	}
	return nil
}

func (a *App) applyResumability(ctx context.Context) error {
	absentLedger, err := findResumeLedger(ctx, a.dataStore, a.config.DataStoreConfig.Schema,
		a.config.StartLedger, a.config.EndLedger)
	if err != nil {
		return err
	}
	if absentLedger == 0 {
		return NewDataAlreadyExportedError(a.config.StartLedger, a.config.EndLedger)
	}

	if absentLedger > 2 && absentLedger != a.config.DataStoreConfig.Schema.GetSequenceNumberStartBoundary(absentLedger) {
		return NewInvalidDataStoreError(absentLedger, a.config.DataStoreConfig.Schema.LedgersPerFile)
	}
	logger.Infof("For export ledger range start=%d, end=%d, the remote storage has some of this data already, will resume at later start ledger of %d",
		a.config.StartLedger, a.config.EndLedger, absentLedger)
	a.config.StartLedger = absentLedger
	return nil
}

func (a *App) close() {
	if err := a.dataStore.Close(); err != nil {
		logger.WithError(err).Error("Error closing datastore")
	}
	if a.config.Mode.Export() {
		if err := a.ledgerBackend.Close(); err != nil {
			logger.WithError(err).Error("Error closing ledgerBackend")
		}
	}
}

func newAdminServer(adminPort int, prometheusRegistry *prometheus.Registry) *http.Server {
	mux := supporthttp.NewMux(logger)
	mux.Handle("/metrics", promhttp.HandlerFor(prometheusRegistry, promhttp.HandlerOpts{}))
	adminAddr := fmt.Sprintf(":%d", adminPort)
	return &http.Server{
		Addr:        adminAddr,
		Handler:     mux,
		ReadTimeout: adminServerReadTimeout,
	}
}

func (a *App) Run(runtimeSettings RuntimeSettings) error {
	ctx, cancel := signal.NotifyContext(runtimeSettings.Ctx, os.Interrupt, syscall.SIGINT, syscall.SIGTERM)
	defer cancel()

	if err := a.init(ctx, runtimeSettings); err != nil {
		var dataAlreadyExported *DataAlreadyExportedError
		if errors.As(err, &dataAlreadyExported) {
			logger.Info(err.Error())
			logger.Info("Shutting down Galexie")
			return nil
		}
		logger.WithError(err).Error("Stopping Galexie")
		return err
	}
	defer a.close()

	switch {
	case runtimeSettings.Mode.Export():
		if err := a.export(ctx, cancel); err != nil {
			logger.WithError(err).Error("Stopping Galexie")
			return err
		}
	case runtimeSettings.Mode == DetectGaps:
		if err := a.runDetectGaps(ctx, runtimeSettings.ReportWriter); err != nil {
			logger.WithError(err).Error("Stopping Galexie")
			return err
		}
	default:
		return fmt.Errorf("unsupported mode: %v", runtimeSettings.Mode)
	}

	logger.Info("Shutting down Galexie")
	return nil
}

func (a *App) export(ctx context.Context, cancel context.CancelFunc) error {
	if a.adminServer != nil {
		go func() {
			logger.Infof("Starting admin server on port %v", a.config.AdminPort)
			if err := a.adminServer.ListenAndServe(); err != nil && !errors.Is(err, http.ErrServerClosed) {
				log.Warn(errors.Wrap(err, "error in internalServer.ListenAndServe()"))
			}
		}()
	}

	var wg sync.WaitGroup
	wg.Add(2)

	go func() {
		defer wg.Done()
		if err := a.uploader.Run(ctx, uploadShutdownTimeout); err != nil {
			logger.WithError(err).Error("Error executing Uploader")
			cancel()
		}
	}()

	go func() {
		defer wg.Done()
		if err := a.exportManager.Run(ctx, a.config.StartLedger, a.config.EndLedger); err != nil {
			if errors.Is(err, loadtest.ErrLoadTestDone) {
				logger.Info("Load test completed.")
			} else {
				logger.WithError(err).Error("Error executing ExportManager")
			}
		}
	}()

	wg.Wait()

	if a.adminServer != nil {
		serverShutdownCtx, serverShutdownCancel := context.WithTimeout(context.Background(), adminServerShutdownTimeout)
		defer serverShutdownCancel()
		if err := a.adminServer.Shutdown(serverShutdownCtx); err != nil {
			logger.WithError(err).Warn("error in internalServer.Shutdown")
		}
	}
	return nil
}

const defaultTaskSize = uint32(100_000)

// TODO: make this configurable
const defaultNumWorkers = 32

func (a *App) runDetectGaps(ctx context.Context, reportWriter io.Writer) error {
	from := a.config.StartLedger
	to   := a.config.EndLedger

	if from > to {
		return fmt.Errorf("invalid range: from (%d) must be <= to (%d)", from, to)
	}

	sc, _ := scan.NewScanner(
		a.dataStore,
		a.config.DataStoreConfig.Schema,
		defaultNumWorkers,
		defaultTaskSize,
		logger,
	)

	start := time.Now()
	rep, err := sc.Run(ctx, from, to)
	dur := time.Since(start)
	if err != nil {
		logger.WithFields(log.F{
			"scan_from": from,
			"scan_to":   to,
		}).WithError(err).Error("detect-gaps scan failed")
		return err
	}

	if reportWriter != nil {
		out := DetectGapsOutput{
			ScanFrom: from,
			ScanTo:   to,
			Duration: dur.String(),
			Report:   rep,
		}
		enc := json.NewEncoder(reportWriter)
		enc.SetIndent("", " ")
		if err := enc.Encode(out); err != nil {
			return fmt.Errorf("failed to encode detect-gaps report: %w", err)
		}
	}

	fields := log.F{
		"scan_from":     from,
		"scan_to":       to,
		"min_found":     rep.MinSequenceFound,
		"max_found":     rep.MaxSequenceFound,
		"total_found":   rep.TotalLedgersFound,
		"total_missing": rep.TotalLedgersMissing,
		"gaps_count":    len(rep.Gaps),
		"duration":      dur.String(),
	}
	if rep.TotalLedgersMissing > 0 {
		logger.WithFields(fields).Warn("detect-gaps completed with gaps")
	} else {
		logger.WithFields(fields).Info("detect-gaps completed successfully")
	}
	return nil
}

func newLedgerBackend(config *Config, prometheusRegistry *prometheus.Registry) (ledgerbackend.LedgerBackend, error) {
	coreBinFromPath, _ := exec.LookPath("stellar-core")
	captiveConfig, err := config.GenerateCaptiveCoreConfig(coreBinFromPath)
	if err != nil {
		return nil, err
	}

	var captiveCoreBackend ledgerbackend.LedgerBackend
	captiveCoreBackend, err = ledgerbackend.NewCaptive(captiveConfig)
	if err != nil {
		return nil, errors.Wrap(err, "Failed to create captive-core instance")
	}

	if config.Mode.LoadTest() {
		captiveCoreBackend = newLoadTestBackend(config, captiveCoreBackend)
	}

	return ledgerbackend.WithMetrics(captiveCoreBackend, prometheusRegistry, nameSpace), nil
}

func newLoadTestBackend(config *Config, backend ledgerbackend.LedgerBackend) *loadtest.LedgerBackend {
	if !config.LoadTestMerge {
		backend = nil
	}
	return loadtest.NewLedgerBackend(loadtest.LedgerBackendConfig{
		NetworkPassphrase:   config.StellarCoreConfig.NetworkPassphrase,
		LedgerBackend:       backend,
		LedgersFilePath:     config.LoadTestLedgersPath,
		LedgerCloseDuration: config.LoadTestCloseDuration,
	})
}
```
