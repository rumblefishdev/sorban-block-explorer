---
prefix: R
title: WASM Contract Spec Extraction - Tools and Approach
status: mature
spawned_from: null
spawns: []
---

# WASM Contract Spec Extraction - Tools and Approach

## Where the Contract Spec Lives

Soroban contract specifications are stored in a **WASM custom section** named `contractspecv0`. This is a standard WASM custom section (section ID 0) with the name encoded as a UTF-8 string. The payload is an **XDR-encoded stream** of `ScSpecEntry` values, serialized back-to-back (not length-prefixed individually -- the entire section is one continuous XDR stream).

### WASM Binary Layout

```
[WASM magic: 0x00 0x61 0x73 0x6D] [version: 0x01 0x00 0x00 0x00]
...standard WASM sections (types, functions, memory, etc.)...
[section_id: 0x00] [section_length: varuint32]
  [name_length: varuint32] [name: "contractspecv0"]
  [payload: XDR stream of ScSpecEntry values]
```

There are two additional custom sections relevant to contract metadata:

- **`contractmetav0`** -- arbitrary key-value metadata (contract name, description, version, author). Format: XDR stream of `SCMetaEntry` (`SCMetaV0` = key/val string pair). Set via `contractmeta!` macro or `stellar contract build --meta`. See [Stellar Docs: WASM Metadata](../sources/stellar-docs-wasm-metadata.md), [Stellar XDR: contract-meta.x](../sources/stellar-xdr-contract-meta.x).
- **`contractenvmetav0`** -- environment compatibility metadata (SDK version, protocol version). Not needed for explorer metadata.

### `contractmetav0` -- Contract Metadata

This section is critical for the explorer because it can provide the **contract name** for `metadata.name` and thus `search_vector`.

```xdr
struct SCMetaV0 {
    string key<>;  // e.g., "Description"
    string val<>;  // e.g., "Constant product AMM with a .3% swap fee"
};

union SCMetaEntry switch (SCMetaKind kind) {
case SC_META_V0:
    SCMetaV0 v0;
};
```

Common keys:
| Key | Set by | Example value | Source |
|-----|--------|---------------|--------|
| `Description` | Developer (`contractmeta!` macro) | "Constant product AMM with a .3% swap fee" | [stellar-docs-wasm-metadata.md](../sources/stellar-docs-wasm-metadata.md) |
| Custom keys | Developer | Any string (name, version, author) | [stellar-docs-wasm-metadata.md](../sources/stellar-docs-wasm-metadata.md) |

**Note**: The Soroban build tooling may also inject additional keys automatically (e.g., Rust compiler version, SDK version). The exact auto-injected keys depend on the build toolchain version and are not formally specified.

**SDK support**: `@stellar/stellar-sdk` does **not** have a high-level API for `contractmetav0` (only `contractspecv0` via `Spec.fromWasm()`). Extraction requires:

```typescript
import { contract } from '@stellar/stellar-sdk';
import { xdr, cereal } from '@stellar/stellar-base';

// 1. Get raw bytes from custom section
const sections = parseWasmCustomSections(wasmBuffer); // internal SDK util
const metaBytes = sections.get('contractmetav0');

// 2. Deserialize XDR stream
if (metaBytes && metaBytes.length > 0) {
  const reader = new cereal.XdrReader(Buffer.from(metaBytes[0]));
  const entries: Array<{ key: string; val: string }> = [];
  while (!reader.eof) {
    const entry = xdr.ScMetaEntry.read(reader);
    entries.push({
      key: entry.v0().key().toString(),
      val: entry.v0().val().toString(),
    });
  }
}
```

**Note**: `parseWasmCustomSections` is exported from the internal `utils.ts` module but not re-exported from the public `@stellar/stellar-sdk/contract` API (verified: not in `contract/index.d.ts`). For implementation, either vendor it (it's ~45 lines of pure JS, see [sdk-source-contract-utils.ts](../sources/sdk-source-contract-utils.ts):100-177) or import from the internal path.

## Recommended Tool: `@stellar/stellar-sdk` (v14.6.1+)

The `@stellar/stellar-sdk` package provides everything needed. No additional WASM parser is required.

### Key API

```typescript
import { contract } from '@stellar/stellar-sdk';
// or: import { Spec } from '@stellar/stellar-sdk/contract';

// From raw WASM bytes:
const spec = contract.Spec.fromWasm(wasmBuffer);

// From pre-extracted XDR (if spec bytes already available):
const spec = new contract.Spec(xdrBuffer);
```

### Internal Pipeline

1. `Spec.fromWasm(buffer)` calls `specFromWasm(buffer)` which calls `parseWasmCustomSections(buffer)`
2. `parseWasmCustomSections` is a **pure JS implementation** that manually parses the WASM binary format -- iterates through sections, finds custom sections (id=0), reads their name, and returns a `Map<string, Uint8Array[]>`
3. The `contractspecv0` section payload is extracted
4. `processSpecEntryStream(buffer)` uses `@stellar/stellar-base`'s XDR reader to decode the stream into `xdr.ScSpecEntry[]`

**No native dependencies required.** The entire pipeline runs in pure JavaScript, making it safe for Lambda/serverless environments.

### `Spec` Class Methods

| Method                       | Returns                          | Purpose                                     |
| ---------------------------- | -------------------------------- | ------------------------------------------- |
| `Spec.fromWasm(buffer)`      | `Spec`                           | Parse WASM binary to spec                   |
| `spec.entries`               | `xdr.ScSpecEntry[]`              | All spec entries (functions, types, events) |
| `spec.funcs()`               | `xdr.ScSpecFunctionV0[]`         | Only function entries                       |
| `spec.getFunc(name)`         | `xdr.ScSpecFunctionV0`           | Single function by name                     |
| `spec.findEntry(name)`       | `xdr.ScSpecEntry`                | Any entry by name                           |
| `spec.errorCases()`          | `xdr.ScSpecUdtErrorEnumCaseV0[]` | Error enum cases                            |
| `spec.jsonSchema(funcName?)` | `JSONSchema7`                    | JSON Schema representation                  |

## ScSpecEntry Data Model

Each `xdr.ScSpecEntry` is a union with these variants:

| Kind                        | Type                   | Contains                                 |
| --------------------------- | ---------------------- | ---------------------------------------- |
| `scSpecEntryFunctionV0`     | `ScSpecFunctionV0`     | Function: name, doc, inputs[], outputs[] |
| `scSpecEntryUdtStructV0`    | `ScSpecUdtStructV0`    | Struct: name, doc, fields[]              |
| `scSpecEntryUdtUnionV0`     | `ScSpecUdtUnionV0`     | Union: name, doc, cases[]                |
| `scSpecEntryUdtEnumV0`      | `ScSpecUdtEnumV0`      | Enum: name, doc, cases[]                 |
| `scSpecEntryUdtErrorEnumV0` | `ScSpecUdtErrorEnumV0` | Error enum: name, doc, cases[]           |
| `scSpecEntryEventV0`        | `ScSpecEventV0`        | Event: name, doc, topics[], data[]       |

### Function Structure (`ScSpecFunctionV0`)

```typescript
{
  doc: string,         // Documentation string
  name: string,        // Function name (e.g., "transfer", "balance")
  inputs: [{
    doc: string,       // Parameter documentation
    name: string,      // Parameter name (e.g., "from", "to", "amount")
    type: ScSpecTypeDef // Parameter type
  }],
  outputs: ScSpecTypeDef[]  // Return type(s)
}
```

### Soroban Type System (`ScSpecTypeDef`)

Primitives:

- `val`, `bool`, `void`, `error`
- `u32`, `i32`, `u64`, `i64`, `u128`, `i128`, `u256`, `i256`
- `timepoint`, `duration`
- `bytes`, `string`, `symbol`
- `address`, `muxedAddress`

Compound:

- `option<T>` -- optional value
- `result<T, E>` -- result with ok/error types
- `vec<T>` -- dynamic array
- `map<K, V>` -- key-value map
- `tuple<T...>` -- fixed-size tuple
- `bytesN` -- fixed-size byte array
- `udt` -- user-defined type (references a struct/union/enum by name)

## Extraction Code Example

```typescript
import { contract, xdr } from '@stellar/stellar-sdk';

function extractContractInterface(wasmBytes: Buffer) {
  const spec = contract.Spec.fromWasm(wasmBytes);

  const functions = spec.funcs().map((fn: xdr.ScSpecFunctionV0) => ({
    name: fn.name().toString(),
    doc: fn.doc().toString(),
    inputs: fn.inputs().map((input: xdr.ScSpecFunctionInputV0) => ({
      name: input.name().toString(),
      doc: input.doc().toString(),
      type: typeDefToString(input.type()),
    })),
    outputs: fn
      .outputs()
      .map((output: xdr.ScSpecTypeDef) => typeDefToString(output)),
  }));

  return { functions };
}

function typeDefToString(typeDef: xdr.ScSpecTypeDef): string {
  const name = typeDef.switch().name;
  switch (name) {
    case 'scSpecTypeVal':
      return 'val';
    case 'scSpecTypeBool':
      return 'bool';
    case 'scSpecTypeVoid':
      return 'void';
    case 'scSpecTypeU32':
      return 'u32';
    case 'scSpecTypeI32':
      return 'i32';
    case 'scSpecTypeU64':
      return 'u64';
    case 'scSpecTypeI64':
      return 'i64';
    case 'scSpecTypeU128':
      return 'u128';
    case 'scSpecTypeI128':
      return 'i128';
    case 'scSpecTypeU256':
      return 'u256';
    case 'scSpecTypeI256':
      return 'i256';
    case 'scSpecTypeTimepoint':
      return 'timepoint';
    case 'scSpecTypeDuration':
      return 'duration';
    case 'scSpecTypeBytes':
      return 'bytes';
    case 'scSpecTypeString':
      return 'string';
    case 'scSpecTypeSymbol':
      return 'symbol';
    case 'scSpecTypeAddress':
      return 'address';
    case 'scSpecTypeMuxedAddress':
      return 'muxedAddress';
    case 'scSpecTypeOption':
      return `Option<${typeDefToString(typeDef.option().valueType())}>`;
    case 'scSpecTypeResult':
      return `Result<${typeDefToString(
        typeDef.result().okType()
      )}, ${typeDefToString(typeDef.result().errorType())}>`;
    case 'scSpecTypeVec':
      return `Vec<${typeDefToString(typeDef.vec().elementType())}>`;
    case 'scSpecTypeMap':
      return `Map<${typeDefToString(
        typeDef.map().keyType()
      )}, ${typeDefToString(typeDef.map().valueType())}>`;
    case 'scSpecTypeTuple':
      return `(${typeDef
        .tuple()
        .valueTypes()
        .map(typeDefToString)
        .join(', ')})`;
    case 'scSpecTypeBytesN':
      return `BytesN<${typeDef.bytesN().n()}>`;
    case 'scSpecTypeUdt':
      return typeDef.udt().name().toString();
    default:
      return name;
  }
}
```

## Performance Assessment

The WASM parsing is **lightweight**:

1. **Custom section scan**: O(n) single pass through WASM binary, skipping non-custom sections by jumping `sectionLength` bytes. No need to parse function bodies, types, or other WASM sections.
2. **XDR deserialization**: Linear scan through the spec payload, which is typically small (a few KB even for complex contracts).
3. **No WASM compilation**: The binary is parsed structurally, not compiled/instantiated.

**Estimated performance**: < 10ms for typical contracts. Even large contracts with many functions should parse in < 50ms. This is well within the ~10 second Lambda budget, especially since contract deployments are infrequent (perhaps 1-5 per ledger at most).

**Memory**: The WASM binary must be loaded into memory as a Buffer. Soroban enforces WASM size limits via network configuration, so binaries are bounded. This is trivial for a Lambda with 128MB+ memory.

## Alternative Tools Evaluated

| Tool                                | Assessment                                                                                                                                                                                                                                           |
| ----------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `@stellar/stellar-sdk` `Spec` class | **Recommended.** Native support, maintained by SDF, pure JS, handles full pipeline.                                                                                                                                                                  |
| `wasmparser` (npm)                  | Unnecessary. General-purpose WASM binary parser that can read custom sections, but requires `@stellar/stellar-base` for XDR deserialization anyway. The SDK's built-in `parseWasmCustomSections` is more targeted (~45 lines vs full module parser). |
| `@aspect-build/wasm-parser`         | Does not exist on npm. Mentioned in task spec but not a real package.                                                                                                                                                                                |
| Custom manual parser                | Unnecessary given the SDK handles it. The SDK's `parseWasmCustomSections` is already a minimal custom parser (~45 lines).                                                                                                                            |
| Rust `wasmparser` via WASM          | Overkill. Would add a native WASM dependency to the Lambda for no benefit.                                                                                                                                                                           |

## Sources

- [SEP-0048: Contract Interface Specification](../sources/sep-0048-contract-interface-spec.md) -- defines `contractspecv0` section format, XDR types, stream encoding
- [Stellar XDR: Stellar-contract-spec.x](../sources/stellar-xdr-contract-spec.x) -- XDR definition of `SCSpecEntry`, `SCSpecFunctionV0`, `SCSpecTypeDef`
- [Stellar XDR: Stellar-contract-meta.x](../sources/stellar-xdr-contract-meta.x) -- XDR definition of `SCMetaV0`, `SCMetaEntry`
- [Stellar Docs: Fully Typed Contracts](../sources/stellar-docs-fully-typed-contracts.md) -- how the WASM custom section encodes interface types
- [Stellar Docs: Build Your Own SDK](../sources/stellar-docs-build-your-own-sdk.md) -- explains contract spec format for SDK builders
- [Stellar Docs: WASM Metadata](../sources/stellar-docs-wasm-metadata.md) -- `contractmetav0` section usage, `contractmeta!` macro
- [@stellar/stellar-sdk on npm](../sources/npm-stellar-sdk.md) -- SDK package with `contract.Spec` class
- [SDK source: `contract/utils.ts`](../sources/sdk-source-contract-utils.ts) -- `parseWasmCustomSections` implementation (pure JS WASM parser)
- [SDK source: `contract/wasm_spec_parser.ts`](../sources/sdk-source-wasm-spec-parser.ts) -- `specFromWasm` implementation
- [SDK source: `contract/spec.ts`](../sources/sdk-source-spec.ts) -- `Spec` class with all methods (`funcs()`, `findEntry()`, `jsonSchema()`, etc.)
