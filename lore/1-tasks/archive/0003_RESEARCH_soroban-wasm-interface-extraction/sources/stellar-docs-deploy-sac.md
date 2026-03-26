---
source_url: 'https://developers.stellar.org/docs/build/guides/conventions/deploy-sac-with-code'
title: 'Deploy a Stellar Asset Contract (SAC) for a Stellar asset using code'
fetched_date: 2026-03-26
task_id: '0003'
---

# Deploy a Stellar Asset Contract (SAC) for a Stellar asset using code

## Overview

This guide teaches developers how to deploy a Stellar Asset Contract for a blockchain asset using the Stellar SDK. The SDK provides tools and libraries for building applications that interact with the Stellar network.

### Prerequisites

- Node.js and npm installed
- Stellar SDK for JavaScript installed
- Knowledge about issuing an asset on Stellar
- Understanding of the `submitTx` function from transaction submission guides

## Code Overview

```javascript
import * as StellarSdk from '@stellar/stellar-sdk';

const networkRPC = 'https://soroban-testnet.stellar.org';
const server = new StellarSdk.rpc.Server(networkRPC);
const networkPassphrase = StellarSdk.Networks.TESTNET;

const deployStellarAssetContract = async () => {
  const sourceSecrets =
    'SASI6PA4K52GQJF6BC263GLYOADVKFJ4SZ7TFX4QQF2U76T3EJ54DT7Y'; // Replace with your Secret Key
  const sourceKeypair = StellarSdk.Keypair.fromSecret(sourceSecrets);
  const sourceAccount = await server.getAccount(sourceKeypair.publicKey());

  try {
    const assetCode = 'JOEBOY';
    const issuerPublicKey = sourceKeypair.publicKey();
    const customAsset = new StellarSdk.Asset(assetCode, issuerPublicKey);

    const transaction = new StellarSdk.TransactionBuilder(sourceAccount, {
      fee: StellarSdk.BASE_FEE,
      networkPassphrase,
    })
      .addOperation(
        StellarSdk.Operation.createStellarAssetContract({
          asset: customAsset,
        })
      )
      .setTimeout(30)
      .build();

    const uploadTx = await server.prepareTransaction(transaction);
    uploadTx.sign(sourceKeypair);

    const feedback = await submitTx(uploadTx);
    const contract = StellarSdk.Address.fromScAddress(
      feedback.returnValue.address()
    );
    console.log(
      `ContractID of Our ${customAsset.code} Asset`,
      contract.toString()
    );
  } catch (e) {
    console.error('An error occurred while Deploying assets:', e);
  }
};

await deployStellarAssetContract();
```

## Code Explanation

**Server Setup**

```javascript
import * as StellarSdk from '@stellar/stellar-sdk';

const networkRPC = 'https://soroban-testnet.stellar.org';
const server = new StellarSdk.rpc.Server(networkRPC);
const networkPassphrase = StellarSdk.Networks.TESTNET;
```

- `networkRPC`: The endpoint URL for the Soroban testnet
- `server`: Creates an RPC server instance for network interaction
- `networkPassphrase`: Configures the network to testnet

**DeployStellarAssetContract Function**

```javascript
const deployStellarAssetContract = async () => {
  const sourceSecrets =
    'SASI6PA4K52GQJF6BC263GLYOADVKFJ4SZ7TFX4QQF2U76T3EJ54DT7Y'; // Replace with your Secret Key
  const sourceKeypair = StellarSdk.Keypair.fromSecret(sourceSecrets);
  const sourceAccount = await server.getAccount(sourceKeypair.publicKey());

  try {
    const assetCode = 'JOEBOY';
    const issuerPublicKey = sourceKeypair.publicKey();
    const customAsset = new StellarSdk.Asset(assetCode, issuerPublicKey);

    const transaction = new StellarSdk.TransactionBuilder(sourceAccount, {
      fee: StellarSdk.BASE_FEE,
      networkPassphrase,
    })
      .addOperation(
        StellarSdk.Operation.createStellarAssetContract({
          asset: customAsset,
        })
      )
      .setTimeout(30)
      .build();

    const uploadTx = await server.prepareTransaction(transaction);
    uploadTx.sign(sourceKeypair);

    const feedback = await submitTx(uploadTx);
    const contract = StellarSdk.Address.fromScAddress(
      feedback.returnValue.address()
    );
    console.log(
      `ContractID of Our ${customAsset.code} Asset`,
      contract.toString()
    );
  } catch (e) {
    console.error('An error occurred while Deploying assets:', e);
  }
};
```

The function orchestrates deployment of a smart contract for a blockchain asset on the testnet:

- **Secret Key Management**: Requires your private key to authenticate operations
- **Account Setup**: Retrieves current account information from the network
- **Asset Definition**: Creates a custom asset with specified code and issuer details
- **Transaction Construction**: Builds a transaction containing the contract creation operation with a 30-second timeout
- **Network Submission**: Prepares and signs the transaction, then sends it to the network
- **Result Handling**: Captures the contract address from the response and displays the contract identifier

## Related Guides

- Cross-contract calls: Invoking smart contracts from other contracts
- Contract deployment from Wasm bytecode using deployer contracts
- Contract error management with enum types
- Extending contract TTL for deployed code
- Upgrading Wasm bytecode for deployed contracts
- Writing metadata for contracts using Rust SDK macros
- Organizing contracts with Cargo workspaces
