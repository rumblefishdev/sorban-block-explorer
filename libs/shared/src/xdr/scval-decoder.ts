import { xdr, Address } from '@stellar/stellar-base';

// --- Decoded ScVal tagged types ---

export type DecodedScVal =
  | { type: 'bool'; value: boolean }
  | { type: 'void' }
  | { type: 'error'; value: string }
  | { type: 'u32'; value: number }
  | { type: 'i32'; value: number }
  | { type: 'u64'; value: string }
  | { type: 'i64'; value: string }
  | { type: 'timepoint'; value: string }
  | { type: 'duration'; value: string }
  | { type: 'u128'; value: string }
  | { type: 'i128'; value: string }
  | { type: 'u256'; value: string }
  | { type: 'i256'; value: string }
  | { type: 'bytes'; value: string }
  | { type: 'string'; value: string }
  | { type: 'symbol'; value: string }
  | { type: 'address'; value: string }
  | { type: 'vec'; value: readonly DecodedScVal[] }
  | {
      type: 'map';
      value: readonly { key: DecodedScVal; val: DecodedScVal }[];
    }
  | {
      type: 'instance';
      executable: string;
      storage: readonly { key: DecodedScVal; val: DecodedScVal }[];
    }
  | { type: 'ledgerKeyContractInstance' }
  | { type: 'ledgerKeyNonce'; value: string }
  | { type: 'unknown'; value: string };

function uint128ToString(parts: xdr.UInt128Parts): string {
  const lo = BigInt('0x' + parts.lo().toXDR('hex'));
  const hi = BigInt('0x' + parts.hi().toXDR('hex'));
  return ((hi << 64n) | lo).toString();
}

function int128ToString(parts: xdr.Int128Parts): string {
  const lo = BigInt('0x' + parts.lo().toXDR('hex'));
  const hi = BigInt(parts.hi().toString());
  return ((hi << 64n) | lo).toString();
}

function uint256ToString(parts: xdr.UInt256Parts): string {
  const hiHi = BigInt('0x' + parts.hiHi().toXDR('hex'));
  const hiLo = BigInt('0x' + parts.hiLo().toXDR('hex'));
  const loHi = BigInt('0x' + parts.loHi().toXDR('hex'));
  const loLo = BigInt('0x' + parts.loLo().toXDR('hex'));
  return ((hiHi << 192n) | (hiLo << 128n) | (loHi << 64n) | loLo).toString();
}

function int256ToString(parts: xdr.Int256Parts): string {
  const hiHi = BigInt(parts.hiHi().toString());
  const hiLo = BigInt('0x' + parts.hiLo().toXDR('hex'));
  const loHi = BigInt('0x' + parts.loHi().toXDR('hex'));
  const loLo = BigInt('0x' + parts.loLo().toXDR('hex'));
  return ((hiHi << 192n) | (hiLo << 128n) | (loHi << 64n) | loLo).toString();
}

function decodeAddress(addr: xdr.ScAddress): string {
  return Address.fromScAddress(addr).toString();
}

function bufferToString(val: string | Buffer): string {
  return typeof val === 'string' ? val : val.toString('utf8');
}

function decodeMapEntries(
  entries: xdr.ScMapEntry[] | null
): readonly { key: DecodedScVal; val: DecodedScVal }[] {
  if (!entries) return [];
  return entries.map((entry) => ({
    key: decodeScVal(entry.key()),
    val: decodeScVal(entry.val()),
  }));
}

export function decodeScVal(scVal: xdr.ScVal): DecodedScVal {
  const type = scVal.switch();

  switch (type.value) {
    case xdr.ScValType.scvBool().value:
      return { type: 'bool', value: scVal.b() };

    case xdr.ScValType.scvVoid().value:
      return { type: 'void' };

    case xdr.ScValType.scvError().value:
      return { type: 'error', value: scVal.error().toXDR('hex') };

    case xdr.ScValType.scvU32().value:
      return { type: 'u32', value: scVal.u32() };

    case xdr.ScValType.scvI32().value:
      return { type: 'i32', value: scVal.i32() };

    case xdr.ScValType.scvU64().value:
      return { type: 'u64', value: scVal.u64().toString() };

    case xdr.ScValType.scvI64().value:
      return { type: 'i64', value: scVal.i64().toString() };

    case xdr.ScValType.scvTimepoint().value:
      return { type: 'timepoint', value: scVal.timepoint().toString() };

    case xdr.ScValType.scvDuration().value:
      return { type: 'duration', value: scVal.duration().toString() };

    case xdr.ScValType.scvU128().value:
      return { type: 'u128', value: uint128ToString(scVal.u128()) };

    case xdr.ScValType.scvI128().value:
      return { type: 'i128', value: int128ToString(scVal.i128()) };

    case xdr.ScValType.scvU256().value:
      return { type: 'u256', value: uint256ToString(scVal.u256()) };

    case xdr.ScValType.scvI256().value:
      return { type: 'i256', value: int256ToString(scVal.i256()) };

    case xdr.ScValType.scvBytes().value:
      return { type: 'bytes', value: scVal.bytes().toString('hex') };

    case xdr.ScValType.scvString().value:
      return { type: 'string', value: bufferToString(scVal.str()) };

    case xdr.ScValType.scvSymbol().value:
      return { type: 'symbol', value: bufferToString(scVal.sym()) };

    case xdr.ScValType.scvAddress().value:
      return { type: 'address', value: decodeAddress(scVal.address()) };

    case xdr.ScValType.scvVec().value:
      return { type: 'vec', value: (scVal.vec() ?? []).map(decodeScVal) };

    case xdr.ScValType.scvMap().value:
      return { type: 'map', value: decodeMapEntries(scVal.map()) };

    case xdr.ScValType.scvContractInstance().value: {
      const instance = scVal.instance();
      const exec = instance.executable();
      const execStr =
        exec.switch().value ===
        xdr.ContractExecutableType.contractExecutableWasm().value
          ? `wasm:${exec.wasmHash().toString('hex')}`
          : 'stellar_asset';
      return {
        type: 'instance',
        executable: execStr,
        storage: decodeMapEntries(instance.storage()),
      };
    }

    case xdr.ScValType.scvLedgerKeyContractInstance().value:
      return { type: 'ledgerKeyContractInstance' };

    case xdr.ScValType.scvLedgerKeyNonce().value:
      return {
        type: 'ledgerKeyNonce',
        value: scVal.nonceKey().nonce().toString(),
      };

    default:
      return { type: 'unknown', value: scVal.toXDR('hex') };
  }
}
