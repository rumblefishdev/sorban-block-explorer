---
url: 'https://docs.aqua.network/developers/code-examples/prerequisites-and-basics'
title: 'Prerequisites & Basics - Aquarius Developer Docs'
fetched_date: 2026-03-26
task_id: '0003'
---

# Prerequisites & Basics

### Prerequisites

Before executing scripts, ensure you have Stellar SDK installed:

**Python**

- **Python 3.7+**: The script is written in Python and requires Python version 3.7 or higher.
- **Stellar SDK**: Install via pip:

```
pip install stellar-sdk
```

**JavaScript**

- **Node 18+**: The script is written in JavaScript and requires Node version 18 or higher.
- **Stellar SDK**: Install via npm or yarn.

```bash
npm install @stellar/stellar-sdk
```

or

```bash
yarn add @stellar/stellar-sdk
```

### Constants

Below are some API endpoints and contract addresses that will be used in other code examples.

#### For Mainnet

**Python**

```python
# The contract ID of the Aquarius AMM contract
router_contract_id = "CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK"
# Soroban RPC server address
soroban_rpc_server = "https://mainnet.sorobanrpc.com"
# Horizon server address
horizon_server = "https://horizon.stellar.org"
# Aquarius backend API URL
base_api = "https://amm-api.aqua.network/api/external/v1"
```

**JavaScript**

```javascript
// The contract ID of the Aquarius AMM contract
const routerContractId =
  'CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK';
// Soroban RPC server address
const sorobanRpcServer = 'https://mainnet.sorobanrpc.com';
// Horizon server address
const horizonServer = 'https://horizon.stellar.org';
// Aquarius backend API URL
const baseApi = 'https://amm-api.aqua.network/api/external/v1';
```

#### For Testnet

**Python**

```python
# The contract ID of the Aquarius AMM contract
# Address updated on February 2026 and should be valid across testnet resets.
router_contract_id = "CBCFTQSPDBAIZ6R6PJQKSQWKNKWH2QIV3I4J72SHWBIK3ADRRAM5A6GD"
# Soroban RPC server address
soroban_rpc_server = "https://soroban-testnet.stellar.org:443"
# Horizon server address
horizon_server = "https://horizon-testnet.stellar.org"
# Aquarius backend API URL
base_api = "https://amm-api-testnet.aqua.network/api/external/v1"
```

**JavaScript**

```javascript
// The contract ID of the Aquarius AMM contract
// Address updated on February 2026 and should be valid across testnet resets.
const routerContractId =
  'CBCFTQSPDBAIZ6R6PJQKSQWKNKWH2QIV3I4J72SHWBIK3ADRRAM5A6GD';
// Soroban RPC server address
const sorobanRpcServer = 'https://soroban-testnet.stellar.org:443';
// Horizon server address
const horizonServer = 'https://horizon-testnet.stellar.org';
// Aquarius backend API URL
const baseApi = 'https://amm-api-testnet.aqua.network/api/external/v1';
```

### Helper Functions

Common utility methods that will be used in other code examples.

#### Get Asset Contract Id

To interact with any Soroban assets, you need their **smart contract address**.

This code snippet demonstrates how to retrieve the contract address of an asset on the Stellar network. The code uses the PUBLIC network by default, but you can switch to the desired network (e.g. TESTNET).

**Python**

```python
from stellar_sdk import Asset, Network

# Create the native asset
asset = Asset.native()
# Or create a custom asset
# asset = Asset("AQUA", "GBNZILSTVQZ4R7IKQDGHYGY2QXL5QOFJYQMXPKWRRM5PAV7Y4M67AQUA")

# Retrieve the contract ID for the PUBLIC network
contract_id = asset.contract_id(Network.PUBLIC_NETWORK_PASSPHRASE)

print(contract_id)
# Example output:
# "CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA"
```

**JavaScript**

```javascript
const StellarSdk = require('@stellar/stellar-sdk');

// Create the native asset
const asset = StellarSdk.Asset.native();
// Or create a custom asset
// const asset = new StellarSdk.Asset('AQUA', 'GBNZILSTVQZ4R7IKQDGHYGY2QXL5QOFJYQMXPKWRRM5PAV7Y4M67AQUA');

// Retrieve the contract ID for the PUBLIC network
const contractId = asset.contractId(StellarSdk.Networks.PUBLIC);

console.log(contractId);
// console.log example
// "CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA"
```

#### Get Pool Contract ID and Pool Hash

To interact with an Aquarius pool (e.g. make deposits or direct swaps), you need the **address of the pool contract**.

One option to find the pool address is to refer to the pool page on the Aquarius website.

In order to receive this information programmatically, please refer to the detailed description in the "Get Pools Info" section of the Aquarius developer docs.

#### Order Tokens IDs

Most of the time, if an array of token IDs needs to be passed as arguments in a contract call, they should be sorted.

**Python**

```python
def order_token_ids(tokens: List[xdr.SCVal]) -> List[xdr.SCVal]:
   """
   Orders token IDs based on their contract ID to maintain consistency.

   Args:
       tokens (List[xdr.SCVal]): List of token addresses as SCVal objects.

   Returns:
       List[xdr.SCVal]: Ordered list of token SCVal objects.
   """
   return sorted(tokens, key=lambda token: int(token.address.contract_id.hash.hex(), 16))
```

**JavaScript**

```javascript
function orderTokensIds(tokensIds) {
  /**
   * Orders token IDs based on their contract ID to maintain consistency.
   *
   * @param {Array} tokensIds - List of token addresses as SCVal objects.
   * @returns {Array} Ordered list of token SCVal objects.
   */
  return tokensIds.sort((a, b) => {
    const aHash = BigInt('0x' + a.address().contractId().toString('hex'));
    const bHash = BigInt('0x' + b.address().contractId().toString('hex'));

    // Compare BigInts directly without converting to number
    if (aHash < bHash) return -1;
    if (aHash > bHash) return 1;
    return 0;
  });
}
```

#### Data Conversion Utilities

Here are methods and code snippets for converting data between basic types and ScVal (Smart Contract Value).

**Python**

```python
from stellar_sdk import Address, scval

# ================
# Basic => ScVal
# ================

# Contract Id To ScVal
contract_id = "C..."
contract_id_scval = scval.to_address(contract_id)

# Array To ScVal
scval.to_vec([contract_id_scv, contract_id_scv])

# Public Key To ScVal
public_key = "G..."
public_key_scval = scval.to_address(public_key)

# Number To Uint32 ScVal
number_u32_scval = scval.to_uint32(1_000_000)

# Number To Uint128 ScVal
number_u128_scval = scval.to_uint128(1_000_000)

# Hash To ScVal
pool_hash = "a1b2...."
pool_hash_scval = scval.to_bytes(bytes.fromhex(pool_hash))

# ================
#  ScVal => Basic
# ================

def u128_to_int(value: UInt128Parts) -> int:
   """
   Converts UInt128Parts from Stellar's XDR to a Python integer.

   Args:
       value (UInt128Parts): UInt128Parts object from Stellar SDK.

   Returns:
       int: Corresponding Python integer.
   """
   return int(value.hi.uint64 << 64) + value.lo.uint64
```

**JavaScript**

```javascript
const StellarSdk = require('@stellar/stellar-sdk');
const binascii = require('binascii');
const { Address, StrKey, XdrLargeInt, xdr } = StellarSdk;

// ================
// Basic => ScVal
// ================

function contractIdToScVal(contractId) {
  return Address.contract(StrKey.decodeContract(contractId)).toScVal();
}

function arrayToScVal(array) {
  return xdr.ScVal.scvVec(array);
}

function publicKeyToScVal(pubkey) {
  return xdr.ScVal.scvAddress(Address.fromString(pubkey).toScAddress());
}

function numberToUint32(number) {
  return xdr.ScVal.scvU32(number);
}

function numberToUint128(number) {
  return new XdrLargeInt('u128', number.toFixed()).toU128();
}

function bufferToScVal(buffer) {
  return xdr.ScVal.scvBytes(buffer);
}

function hashToScVal(hash) {
  return xdr.ScVal.scvBytes(Buffer.from(binascii.unhexlify(hash), 'ascii'));
}

// ================
// ScVal => Basic
// ================

function u128ToInt(value) {
  /**
   * Converts UInt128Parts from Stellar's XDR to a JavaScript number.
   *
   * @param {Object} value - UInt128Parts object from Stellar SDK, with `hi` and `lo` properties.
   * @returns {number|null} Corresponding JavaScript number, or null if the number is too large.
   */
  const result = (BigInt(value.hi()._value) << 64n) + BigInt(value.lo()._value);

  if (result <= BigInt(Number.MAX_SAFE_INTEGER)) {
    return Number(result);
  } else {
    console.warn("Value exceeds JavaScript's safe integer range");
    return null;
  }
}
```
