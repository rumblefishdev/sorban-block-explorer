import { xdr } from '@stellar/stellar-base';
import { Spec } from '@stellar/stellar-sdk/contract';

export interface ContractFunction {
  name: string;
  parameters: readonly { name: string; type: string }[];
  returnType: string;
}

function bufToString(val: string | Buffer): string {
  return typeof val === 'string' ? val : val.toString('utf8');
}

function specTypeToString(typeDef: xdr.ScSpecTypeDef): string {
  const type = typeDef.switch();
  const name = type.name;

  // Simple types — strip the "scSpecType" prefix and lowercase
  const simpleTypes: Record<string, string> = {
    scSpecTypeVal: 'val',
    scSpecTypeBool: 'bool',
    scSpecTypeVoid: 'void',
    scSpecTypeError: 'error',
    scSpecTypeU32: 'u32',
    scSpecTypeI32: 'i32',
    scSpecTypeU64: 'u64',
    scSpecTypeI64: 'i64',
    scSpecTypeTimepoint: 'timepoint',
    scSpecTypeDuration: 'duration',
    scSpecTypeU128: 'u128',
    scSpecTypeI128: 'i128',
    scSpecTypeU256: 'u256',
    scSpecTypeI256: 'i256',
    scSpecTypeBytes: 'bytes',
    scSpecTypeString: 'string',
    scSpecTypeSymbol: 'symbol',
    scSpecTypeAddress: 'address',
  };

  const simple = simpleTypes[name];
  if (simple) {
    return simple;
  }

  // Composite types
  switch (name) {
    case 'scSpecTypeOption':
      return `Option<${specTypeToString(typeDef.option().valueType())}>`;
    case 'scSpecTypeResult':
      return `Result<${specTypeToString(
        typeDef.result().okType()
      )}, ${specTypeToString(typeDef.result().errorType())}>`;
    case 'scSpecTypeVec':
      return `Vec<${specTypeToString(typeDef.vec().elementType())}>`;
    case 'scSpecTypeMap':
      return `Map<${specTypeToString(
        typeDef.map().keyType()
      )}, ${specTypeToString(typeDef.map().valueType())}>`;
    case 'scSpecTypeTuple': {
      const types = typeDef.tuple().valueTypes().map(specTypeToString);
      return `(${types.join(', ')})`;
    }
    case 'scSpecTypeBytesN':
      return `BytesN<${String(typeDef.bytesN().n())}>`;
    case 'scSpecTypeUdt':
      return bufToString(typeDef.udt().name());
    default:
      return name;
  }
}

function mapFuncs(funcs: xdr.ScSpecFunctionV0[]): ContractFunction[] {
  return funcs.map((fn) => {
    const name = bufToString(fn.name());
    const parameters = fn.inputs().map((input) => ({
      name: bufToString(input.name()),
      type: specTypeToString(input.type()),
    }));
    const outputs = fn.outputs();
    const returnType =
      outputs.length === 0
        ? 'void'
        : outputs.length === 1
        ? specTypeToString(outputs[0] as xdr.ScSpecTypeDef)
        : `(${outputs.map(specTypeToString).join(', ')})`;

    return { name, parameters, returnType };
  });
}

/**
 * Extract public function signatures from contract WASM bytes.
 */
export function extractContractInterface(
  wasmBytes: Buffer
): ContractFunction[] {
  const spec = Spec.fromWasm(wasmBytes);
  return mapFuncs(spec.funcs());
}

/**
 * Extract public function signatures from spec entries.
 * Useful when spec entries are already available (e.g., from ContractCodeEntry).
 */
export function extractContractInterfaceFromEntries(
  entries: xdr.ScSpecEntry[]
): ContractFunction[] {
  const spec = new Spec(entries);
  return mapFuncs(spec.funcs());
}
