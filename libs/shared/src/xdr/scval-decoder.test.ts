import { describe, it, expect } from 'vitest';
import { xdr, Keypair, Address } from '@stellar/stellar-base';
import { decodeScVal } from './scval-decoder.js';

describe('decodeScVal', () => {
  it('decodes bool', () => {
    const val = xdr.ScVal.scvBool(true);
    expect(decodeScVal(val)).toEqual({ type: 'bool', value: true });
  });

  it('decodes void', () => {
    const val = xdr.ScVal.scvVoid();
    expect(decodeScVal(val)).toEqual({ type: 'void' });
  });

  it('decodes u32', () => {
    const val = xdr.ScVal.scvU32(42);
    expect(decodeScVal(val)).toEqual({ type: 'u32', value: 42 });
  });

  it('decodes i32', () => {
    const val = xdr.ScVal.scvI32(-100);
    expect(decodeScVal(val)).toEqual({ type: 'i32', value: -100 });
  });

  it('decodes u64', () => {
    const val = xdr.ScVal.scvU64(new xdr.Uint64(1000000));
    const result = decodeScVal(val);
    expect(result.type).toBe('u64');
    expect(result).toHaveProperty('value', '1000000');
  });

  it('decodes i64', () => {
    const val = xdr.ScVal.scvI64(new xdr.Int64(999));
    const result = decodeScVal(val);
    expect(result.type).toBe('i64');
    expect(result).toHaveProperty('value', '999');
  });

  it('decodes string', () => {
    const val = xdr.ScVal.scvString('hello');
    expect(decodeScVal(val)).toEqual({ type: 'string', value: 'hello' });
  });

  it('decodes symbol', () => {
    const val = xdr.ScVal.scvSymbol('transfer');
    expect(decodeScVal(val)).toEqual({ type: 'symbol', value: 'transfer' });
  });

  it('decodes bytes as hex', () => {
    const val = xdr.ScVal.scvBytes(Buffer.from([0xde, 0xad, 0xbe, 0xef]));
    expect(decodeScVal(val)).toEqual({ type: 'bytes', value: 'deadbeef' });
  });

  it('decodes vec recursively', () => {
    const val = xdr.ScVal.scvVec([xdr.ScVal.scvU32(1), xdr.ScVal.scvU32(2)]);
    const result = decodeScVal(val);
    expect(result.type).toBe('vec');
    if (result.type === 'vec') {
      expect(result.value).toHaveLength(2);
      expect(result.value[0]).toEqual({ type: 'u32', value: 1 });
      expect(result.value[1]).toEqual({ type: 'u32', value: 2 });
    }
  });

  it('decodes map recursively', () => {
    const val = xdr.ScVal.scvMap([
      new xdr.ScMapEntry({
        key: xdr.ScVal.scvSymbol('name'),
        val: xdr.ScVal.scvString('Alice'),
      }),
    ]);
    const result = decodeScVal(val);
    expect(result.type).toBe('map');
    if (result.type === 'map') {
      expect(result.value).toHaveLength(1);
      expect(result.value[0]?.key).toEqual({ type: 'symbol', value: 'name' });
      expect(result.value[0]?.val).toEqual({
        type: 'string',
        value: 'Alice',
      });
    }
  });

  it('decodes ledgerKeyContractInstance', () => {
    const val = xdr.ScVal.scvLedgerKeyContractInstance();
    expect(decodeScVal(val)).toEqual({ type: 'ledgerKeyContractInstance' });
  });

  it('handles empty vec', () => {
    const val = xdr.ScVal.scvVec([]);
    const result = decodeScVal(val);
    expect(result.type).toBe('vec');
    if (result.type === 'vec') {
      expect(result.value).toHaveLength(0);
    }
  });

  it('handles empty map', () => {
    const val = xdr.ScVal.scvMap([]);
    const result = decodeScVal(val);
    expect(result.type).toBe('map');
    if (result.type === 'map') {
      expect(result.value).toHaveLength(0);
    }
  });

  it('decodes address (account)', () => {
    const keypair = Keypair.random();
    const addr = new Address(keypair.publicKey());
    const val = xdr.ScVal.scvAddress(addr.toScAddress());
    const result = decodeScVal(val);
    expect(result).toEqual({ type: 'address', value: keypair.publicKey() });
  });

  it('decodes address (contract)', () => {
    const contractBuf = Buffer.alloc(32, 0xab);
    const addr = Address.contract(contractBuf);
    const val = xdr.ScVal.scvAddress(addr.toScAddress());
    const result = decodeScVal(val);
    expect(result.type).toBe('address');
    if (result.type === 'address') {
      expect(result.value).toBe(addr.toString());
      expect(result.value).toMatch(/^C[A-Z0-9]+$/);
    }
  });

  it('decodes u128', () => {
    const val = xdr.ScVal.scvU128(
      new xdr.UInt128Parts({
        lo: new xdr.Uint64(100),
        hi: new xdr.Uint64(0),
      })
    );
    const result = decodeScVal(val);
    expect(result.type).toBe('u128');
    expect(result).toHaveProperty('value', '100');
  });

  it('decodes i128 negative value', () => {
    // i128 = -1 is represented as hi=-1, lo=max_u64
    const val = xdr.ScVal.scvI128(
      new xdr.Int128Parts({
        lo: xdr.Uint64.fromString('18446744073709551615'),
        hi: new xdr.Int64(-1),
      })
    );
    const result = decodeScVal(val);
    expect(result.type).toBe('i128');
    expect(result).toHaveProperty('value', '-1');
  });
});
