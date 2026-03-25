---
url: 'https://pkg.go.dev/github.com/stellar/go/network'
title: 'network package - github.com/stellar/go/network'
fetched_date: 2026-03-25
task_id: '0001'
overwritten: false
image_count: 0
---

# network package — github.com/stellar/go/network

Source: `https://pkg.go.dev/github.com/stellar/go/network`

---

## Constants

```go
const (
    // PublicNetworkPassphrase is the pass phrase used for every transaction
    // intended for the public stellar network
    PublicNetworkPassphrase = "Public Global Stellar Network ; September 2015"

    // TestNetworkPassphrase is the pass phrase used for every transaction
    // intended for the SDF-run test network
    TestNetworkPassphrase = "Test SDF Network ; September 2015"

    // FutureNetworkPassphrase is the pass phrase used for every transaction
    // intended for the SDF-run future network
    FutureNetworkPassphrase = "Test SDF Future Network ; October 2022"
)
```

---

## Variables — History Archive URLs

```go
var (
    PublicNetworkhistoryArchiveURLs = []string{
        "https://history.stellar.org/prd/core-live/core_live_001/",
        "https://history.stellar.org/prd/core-live/core_live_002/",
        "https://history.stellar.org/prd/core-live/core_live_003/",
    }

    TestNetworkhistoryArchiveURLs = []string{
        "https://history.stellar.org/prd/core-testnet/core_testnet_001/",
        "https://history.stellar.org/prd/core-testnet/core_testnet_002/",
        "https://history.stellar.org/prd/core-testnet/core_testnet_003",
    }

    FutureNetworkhistoryArchiveURLs = []string{
        "http://history.stellar.org/dev/core-futurenet/core_futurenet_001/",
        "http://history.stellar.org/dev/core-futurenet/core_futurenet_002/",
        "http://history.stellar.org/dev/core-futurenet/core_futurenet_003/",
    }
)
```

---

## Notes

- Futurenet (also called "Future Network") uses the passphrase `"Test SDF Future Network ; October 2022"`, distinct from Testnet's `"Test SDF Network ; September 2015"`.
- These passphrases are used both as the seed for the root account at genesis and to build transaction hashes that are signed. A transaction signed for one network is invalid on any other.
