---
url: 'https://www.npmjs.com/package/@stellar/stellar-sdk'
title: '@stellar/stellar-sdk - npm'
fetched_date: 2026-03-26
task_id: '0003'
author: 'Stellar Development Foundation'
---

# @stellar/stellar-sdk

> Note: The npmjs.com page returned HTTP 403. Content was sourced from the GitHub repository (https://github.com/stellar/js-stellar-sdk) and supplementary package metadata.

**Package:** `@stellar/stellar-sdk`
**Latest version:** 14.6.1
**License:** Apache-2.0
**Weekly downloads:** listed on npm
**GitHub stars:** 684
**Publisher:** Stellar Development Foundation

Install:

```shell
npm install --save @stellar/stellar-sdk
# or
yarn add @stellar/stellar-sdk
```

## Description

`js-stellar-sdk` is a JavaScript library for communicating with a [Stellar Horizon server](https://developers.stellar.org/docs/data/apis/horizon) and [Stellar RPC](https://developers.stellar.org/docs/data/apis/rpc). While primarily intended for applications built on Node.js or in the browser, it can be adapted for use in other environments with some tinkering.

The library provides:

- a networking layer API for Horizon endpoints (REST-based)
- a networking layer for Soroban RPC (JSONRPC-based)
- facilities for building and signing transactions, for communicating with a Stellar Horizon instance, and for submitting transactions or querying network history

## Installation

Using npm or yarn to include `stellar-sdk` in your own project:

```shell
npm install --save @stellar/stellar-sdk
# or
yarn add @stellar/stellar-sdk
```

Then, require or import it in your JavaScript code:

```javascript
var StellarSdk = require('@stellar/stellar-sdk');
// or
import * as StellarSdk from '@stellar/stellar-sdk';
```

(Preferably, you would only import the pieces you need to enable tree-shaking and lower your final bundle sizes.)

### Browsers

You can use a CDN:

```html
<script src="https://cdnjs.cloudflare.com/ajax/libs/stellar-sdk/{version}/stellar-sdk.js"></script>
```

Note that this method relies on using a third party to host the JS library. This may not be entirely secure. You can self-host it via [Bower](http://bower.io):

```shell
bower install @stellar/stellar-sdk
```

and include it in the browser:

```html
<script src="./bower_components/stellar-sdk/stellar-sdk.js"></script>
<script>
  console.log(StellarSdk);
</script>
```

### Custom Installation

You can configure whether or not to build the browser bundle with the axios dependency. In order to turn off the axios dependency, set the USE_AXIOS environment variable to false. You can also turn off the eventsource dependency by setting USE_EVENTSOURCE to false.

#### Build without Axios

```
npm run build:browser:no-axios
```

This will create `stellar-sdk-no-axios.js` in `dist/`.

#### Build without EventSource

```
npm run build:browser:no-eventsource
```

This will create `stellar-sdk-no-eventsource.js` in `dist/`.

#### Build without Axios and Eventsource

```
npm run build:browser:minimal
```

This will create `stellar-sdk-minimal.js` in `dist/`.

## Usage

The usage documentation for this library lives in a handful of places:

- across the [Stellar Developer Docs](https://developers.stellar.org), which includes tutorials and examples
- within [this repository itself](https://github.com/stellar/js-stellar-sdk/blob/master/docs/reference/readme.md)
- on the generated [API doc site](https://stellar.github.io/js-stellar-sdk/)

You can also refer to:

- the [documentation](https://developers.stellar.org/docs/data/horizon) for the Horizon REST API (if using the `Horizon` module)
- the [documentation](https://developers.stellar.org/docs/data/rpc) for Soroban RPC's API (if using the `rpc` module)

### Usage with React-Native

1. Install `yarn add --dev rn-nodeify`
2. Add the following postinstall script:

```
yarn rn-nodeify --install url,events,https,http,util,stream,crypto,vm,buffer --hack --yarn
```

3. Uncomment `require('crypto')` on shim.js
4. `react-native link react-native-randombytes`
5. Create file `rn-cli.config.js`

```javascript
module.exports = {
  resolver: {
    extraNodeModules: require('node-libs-react-native'),
  },
};
```

6. Add `import "./shim";` to the top of `index.js`
7. `yarn add @stellar/stellar-sdk`

**Note**: Only the V8 compiler (on Android) and JSC (on iOS) have proper support for `Buffer` and `Uint8Array` as is needed by this library. Otherwise, you may see bizarre errors when doing XDR encoding/decoding such as `source not specified`.

### Usage with Expo managed workflows

1. Install `yarn add --dev rn-nodeify`
2. Add the following postinstall script:

```
yarn rn-nodeify --install process,url,events,https,http,util,stream,crypto,vm,buffer --hack --yarn
```

3. Add `import "./shim";` to your app's entry point (by default `./App.js`)
4. `yarn add @stellar/stellar-sdk`
5. `expo install expo-random`

At this point, the Stellar SDK will work, except that `StellarSdk.Keypair.random()` will throw an error. To work around this, you can create your own method to generate a random keypair like this:

```javascript
import * as Random from 'expo-random';
import { Keypair } from '@stellar/stellar-sdk';

const generateRandomKeypair = () => {
  const randomBytes = Random.getRandomBytes(32);
  return Keypair.fromRawEd25519Seed(Buffer.from(randomBytes));
};
```

### Usage with CloudFlare Workers

Both `eventsource` (needed for streaming) and `axios` (needed for making HTTP requests) are problematic dependencies in the CFW environment.

In summary, the `package.json` tweaks look something like this:

```json
"dependencies": {
  "@stellar/stellar-sdk": "git+https://github.com/stellar/js-stellar-sdk#make-eventsource-optional",
  "@vespaiach/axios-fetch-adapter": "^0.3.1",
  "axios": "^0.26.1"
},
"overrides": {
  "@stellar/stellar-sdk": {
    "axios": "$axios"
  }
},
"packageManager": "yarn@1.22.19"
```

Then, you need to override the adapter in your codebase:

```typescript
import { Horizon } from '@stellar/stellar-sdk';
import fetchAdapter from '@vespaiach/axios-fetch-adapter';

Horizon.AxiosClient.defaults.adapter = fetchAdapter as any;

// then, the rest of your code...
```

## CLI

The SDK includes a command-line tool for generating TypeScript bindings from Stellar smart contracts. These bindings provide fully-typed client code with IDE autocompletion and compile-time type checking.

### Running the CLI

```shell
# Using npx (no installation required)
npx @stellar/stellar-sdk generate [options]

# Or if installed globally
stellar-js generate [options]
```

### Generating Bindings

You can generate bindings from three different sources:

#### From a local WASM file

```shell
npx @stellar/stellar-sdk generate \
 --wasm ./path/to/wasm_file/my_contract.wasm \
 --output-dir ./my-contract-client \
 --contract-name my-contract
```

#### From a WASM hash on the network

```shell
npx @stellar/stellar-sdk generate \
 --wasm-hash <hex-encoded-hash> \
 --network testnet \
 --output-dir ./my-contract-client \
 --contract-name my-contract
```

#### From a deployed contract ID

```shell
npx @stellar/stellar-sdk generate \
 --contract-id CABC...XYZ \
 --network testnet \
 --output-dir ./my-contract-client
```

#### With custom RPC server options

```shell
# Mainnet requires --rpc-url (no default)
npx @stellar/stellar-sdk generate \
 --contract-id CABC...XYZ \
 --rpc-url https://my-rpc-provider.com \
 --network mainnet \
 --output-dir ./my-contract-client

# With custom timeout and headers for authenticated RPC servers
npx @stellar/stellar-sdk generate \
 --contract-id CABC...XYZ \
 --rpc-url https://my-rpc-server.com \
 --network mainnet \
 --output-dir ./my-contract-client \
 --timeout 30000 \
 --headers '{"Authorization": "Bearer my-token"}'
```

### CLI Options

| Option                   | Description                                                                                     |
| ------------------------ | ----------------------------------------------------------------------------------------------- |
| `--wasm <path>`          | Path to a local WASM file                                                                       |
| `--wasm-hash <hash>`     | Hex-encoded hash of WASM blob on the network                                                    |
| `--contract-id <id>`     | Contract ID of a deployed contract                                                              |
| `--rpc-url <url>`        | Stellar RPC server URL (has defaults for testnet/futurenet/localnet, required for mainnet)      |
| `--network <network>`    | Network to use: `testnet`, `mainnet`, `futurenet`, or `localnet` (required for network sources) |
| `--output-dir <dir>`     | Output directory for generated bindings (required)                                              |
| `--contract-name <name>` | Name for the generated package (derived from filename if not provided)                          |
| `--overwrite`            | Overwrite existing files in the output directory                                                |
| `--allow-http`           | Allow insecure HTTP connections to RPC server (default: false)                                  |
| `--timeout <ms>`         | RPC request timeout in milliseconds                                                             |
| `--headers <json>`       | Custom headers as JSON object (e.g., `'{"Authorization": "Bearer token"}'`)                     |

### Default RPC URLs

| Network     | Default RPC URL                       |
| ----------- | ------------------------------------- |
| `testnet`   | `https://soroban-testnet.stellar.org` |
| `futurenet` | `https://rpc-futurenet.stellar.org`   |
| `localnet`  | `http://localhost:8000/rpc`           |
| `mainnet`   | None - you must provide `--rpc-url`   |

### Generated Output

The CLI generates a complete npm package structure:

```
my-contract-client/
├── src/
│ ├── index.ts      # Barrel exports
│ ├── client.ts     # Typed Client class with contract methods
│ └── types.ts      # TypeScript interfaces for contract types
├── package.json
├── tsconfig.json
├── README.md
└── .gitignore
```

### Using Generated Bindings

```typescript
import { Client } from './my-contract-client';

const client = new Client({
  contractId: 'CABC...XYZ',
  networkPassphrase: Networks.TESTNET,
  rpcUrl: 'https://soroban-testnet.stellar.org',
  publicKey: keypair.publicKey(),
  ...basicNodeSigner(keypair, Networks.TESTNET),
});

// Fully typed method calls with IDE autocompletion
const result = await client.transfer({
  from: 'GABC...',
  to: 'GDEF...',
  amount: 1000n,
});
```

## stellar-sdk vs stellar-base

`stellar-sdk` is a high-level library that serves as client-side API for Horizon and Soroban RPC, while `stellar-base` is a lower-level library for creating Stellar primitive constructs via XDR helpers and wrappers.

## Dependencies

- `@stellar/stellar-base` - 14.1.0
- `axios` - 1.13.6
- `bignumber.js` - 9.3.1
- `commander` - 14.0.3
- `eventsource` - 2.0.2
- `feaxios` - 0.0.23
- `randombytes` - 2.1.0
- `toml` - 3.0.0
- `urijs` - 1.19.11

## Resources

- [GitHub Repository](https://github.com/stellar/js-stellar-sdk)
- [API Documentation](https://stellar.github.io/js-stellar-sdk/)
- [Stellar Developer Docs](https://developers.stellar.org)
- [Contribution Guide](https://github.com/stellar/js-stellar-sdk/blob/master/CONTRIBUTING.md)
