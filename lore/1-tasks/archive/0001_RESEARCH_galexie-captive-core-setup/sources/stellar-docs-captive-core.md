---
url: 'https://developers.stellar.org/docs/data/indexers/build-your-own/ingest-sdk/developer_guide/ledgerbackends/captivecore'
title: 'Captive Core | Stellar Docs'
fetched_date: 2026-03-25
task_id: '0001'
---

# Captive Core

Captive Core invokes the `stellar-core` binary as a subprocess to stream ledgers from the Stellar network. It can be used to stream a ledger range from the past or to stream new ledgers whenever they are confirmed by the network.

## Prerequisites

Using Captive Core requires the `stellar-core` binary to be [installed](https://developers.stellar.org/docs/validators/admin-guide/installation) first.

### Installation

1. Install Stellar Core:

   - **Option 1**: Build from source by following the [Installation Guide](https://developers.stellar.org/docs/validators/admin-guide/installation#installing-from-source)
   - **Option 2**: Install via a package manager by referring to the [Package-based Installation Guide](https://developers.stellar.org/docs/validators/admin-guide/installation#package-based-installation)

2. Verify installation:

```bash
./stellar-core version
```

## Configuration and Usage

Set the Captive Core configuration for the target Stellar network in TOML format. This configuration requires at a minimum:

- The **passphrase** of the Stellar network you want to connect to.
- The path to **history archives**, necessary for initialization.

### Step 1: Generate a `CaptiveCoreToml` Configuration

```go
captiveCoreToml, err := ledgerbackend.NewCaptiveCoreTomlFromData(
    ledgerbackend.PubnetDefaultConfig,
    ledgerbackend.CaptiveCoreTomlParams{
        NetworkPassphrase:  network.PublicNetworkPassphrase,
        HistoryArchiveURLs: network.PublicNetworkhistoryArchiveURLs,
    },
)

if err != nil {
    // Handle error
}
```

### Step 2: Construct a `CaptiveCoreConfig` Object

Create a `CaptiveCoreConfig` object. This object combines the `CaptiveCoreToml` configuration with the path to your `stellar-core` binary and other necessary parameters.

```go
config := ledgerbackend.CaptiveCoreConfig{
    BinaryPath:         "/usr/local/bin/stellar-core", // Adjust to your stellar-core binary path
    NetworkPassphrase:  network.PublicNetworkPassphrase,
    HistoryArchiveURLs: network.PublicNetworkhistoryArchiveURLs,
    Toml:               captiveCoreToml,
}
```

### Step 3: Instantiate `CaptiveStellarCore`

Create a `CaptiveStellarCore` instance using the `NewCaptive` function. This function manages the complete lifecycle of the `stellar-core` process, including communication with your application.

```go
captiveStellarCoreBackend, err := ledgerbackend.NewCaptive(config)
if err != nil {
    // Handle error
}
```

The `captiveStellarCoreBackend` can now be used to retrieve ledger data within a specified range. For detailed usage, refer to the [code samples](https://developers.stellar.org/docs/data/indexers/build-your-own/ingest-sdk/examples).
