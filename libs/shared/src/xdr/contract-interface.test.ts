import { describe, it, expect } from 'vitest';
import { xdr } from '@stellar/stellar-base';
import { extractContractInterfaceFromEntries } from './contract-interface.js';

function buildSpecEntries(
  fns: {
    name: string;
    inputs: { name: string; type: xdr.ScSpecTypeDef }[];
    outputs: xdr.ScSpecTypeDef[];
  }[]
): xdr.ScSpecEntry[] {
  return fns.map((fn) =>
    xdr.ScSpecEntry.scSpecEntryFunctionV0(
      new xdr.ScSpecFunctionV0({
        doc: '',
        name: fn.name,
        inputs: fn.inputs.map(
          (i) =>
            new xdr.ScSpecFunctionInputV0({
              doc: '',
              name: i.name,
              type: i.type,
            })
        ),
        outputs: fn.outputs,
      })
    )
  );
}

describe('extractContractInterfaceFromEntries', () => {
  it('extracts function names, parameters, and return types', () => {
    const entries = buildSpecEntries([
      {
        name: 'transfer',
        inputs: [
          { name: 'from', type: xdr.ScSpecTypeDef.scSpecTypeAddress() },
          { name: 'to', type: xdr.ScSpecTypeDef.scSpecTypeAddress() },
          { name: 'amount', type: xdr.ScSpecTypeDef.scSpecTypeI128() },
        ],
        outputs: [xdr.ScSpecTypeDef.scSpecTypeBool()],
      },
      {
        name: 'balance',
        inputs: [{ name: 'id', type: xdr.ScSpecTypeDef.scSpecTypeAddress() }],
        outputs: [xdr.ScSpecTypeDef.scSpecTypeI128()],
      },
    ]);

    const result = extractContractInterfaceFromEntries(entries);

    expect(result).toHaveLength(2);
    expect(result[0]?.name).toBe('transfer');
    expect(result[0]?.parameters).toEqual([
      { name: 'from', type: 'address' },
      { name: 'to', type: 'address' },
      { name: 'amount', type: 'i128' },
    ]);
    expect(result[0]?.returnType).toBe('bool');

    expect(result[1]?.name).toBe('balance');
    expect(result[1]?.parameters).toEqual([{ name: 'id', type: 'address' }]);
    expect(result[1]?.returnType).toBe('i128');
  });

  it('returns void for functions with no outputs', () => {
    const entries = buildSpecEntries([
      { name: 'init', inputs: [], outputs: [] },
    ]);

    const result = extractContractInterfaceFromEntries(entries);

    expect(result).toHaveLength(1);
    expect(result[0]?.name).toBe('init');
    expect(result[0]?.parameters).toEqual([]);
    expect(result[0]?.returnType).toBe('void');
  });

  it('maps composite types correctly', () => {
    const entries = buildSpecEntries([
      {
        name: 'get_items',
        inputs: [
          {
            name: 'keys',
            type: xdr.ScSpecTypeDef.scSpecTypeVec(
              new xdr.ScSpecTypeVec({
                elementType: xdr.ScSpecTypeDef.scSpecTypeSymbol(),
              })
            ),
          },
        ],
        outputs: [
          xdr.ScSpecTypeDef.scSpecTypeOption(
            new xdr.ScSpecTypeOption({
              valueType: xdr.ScSpecTypeDef.scSpecTypeString(),
            })
          ),
        ],
      },
    ]);

    const result = extractContractInterfaceFromEntries(entries);

    expect(result).toHaveLength(1);
    expect(result[0]?.parameters).toEqual([
      { name: 'keys', type: 'Vec<symbol>' },
    ]);
    expect(result[0]?.returnType).toBe('Option<string>');
  });

  it('maps Map and Result types', () => {
    const entries = buildSpecEntries([
      {
        name: 'lookup',
        inputs: [
          {
            name: 'data',
            type: xdr.ScSpecTypeDef.scSpecTypeMap(
              new xdr.ScSpecTypeMap({
                keyType: xdr.ScSpecTypeDef.scSpecTypeSymbol(),
                valueType: xdr.ScSpecTypeDef.scSpecTypeU64(),
              })
            ),
          },
        ],
        outputs: [
          xdr.ScSpecTypeDef.scSpecTypeResult(
            new xdr.ScSpecTypeResult({
              okType: xdr.ScSpecTypeDef.scSpecTypeU32(),
              errorType: xdr.ScSpecTypeDef.scSpecTypeError(),
            })
          ),
        ],
      },
    ]);

    const result = extractContractInterfaceFromEntries(entries);

    expect(result[0]?.parameters).toEqual([
      { name: 'data', type: 'Map<symbol, u64>' },
    ]);
    expect(result[0]?.returnType).toBe('Result<u32, error>');
  });
});
