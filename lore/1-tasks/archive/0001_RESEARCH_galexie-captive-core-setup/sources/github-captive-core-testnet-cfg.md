---
url: 'https://github.com/stellar/go-stellar-sdk/blob/main/ingest/ledgerbackend/configs/captive-core-testnet.cfg'
title: 'Captive Core Testnet Configuration'
fetched_date: 2026-03-25
task_id: '0001'
---

# Captive Core Testnet Configuration

**Source:** https://github.com/stellar/go-stellar-sdk/blob/main/ingest/ledgerbackend/configs/captive-core-testnet.cfg
**Fetched:** 2026-03-25

---

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

---

## Notes

This configuration file establishes parameters for Stellar's testnet Captive Core instance. Key settings:

- **Network passphrase:** `Test SDF Network ; September 2015`
- **`UNSAFE_QUORUM=true`** — required for testnet since it has fewer validators than mainnet
- **`FAILURE_SAFETY=1`** — allows the network to progress even if one validator is down
- Three SDF-operated testnet validators (`sdf_testnet_1`, `sdf_testnet_2`, `sdf_testnet_3`)
- All validators use `testnet.stellar.org` as home domain
- History archives are hosted at `history.stellar.org/prd/core-testnet/`
