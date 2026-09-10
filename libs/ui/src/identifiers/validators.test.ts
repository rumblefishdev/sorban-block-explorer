import { describe, expect, it } from 'vitest';

import {
  isAccountId,
  isAssetId,
  isContractId,
  isLedgerSequence,
  isPoolId,
  isPoolIdentifier,
  isTransactionHash,
} from './validators.js';

const VALID_ACCOUNT =
  'GDQP2KPQGKIHYJGXNUIYOMHARUARCA7DJT5FO2FFOOUJ3K4MOMNGEE36';
const VALID_CONTRACT =
  'CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC';
const VALID_POOL = 'LBJRKPHNDQX6FMT2BIPW5ELSZAHOV4DKRY7GNU3CJQX6FMT2BIPW5ELS';
const VALID_TX =
  '7b2a8c1f9d4e6a3b8c1f9d4e6a3b8c1f9d4e6a3b8c1f9d4e6a3b8c1f9d4e6a3b';

describe('isAccountId', () => {
  it('accepts a valid CAP-23 G-strkey', () => {
    expect(isAccountId(VALID_ACCOUNT)).toBe(true);
  });

  it('rejects strkeys with the wrong prefix', () => {
    expect(isAccountId(VALID_CONTRACT)).toBe(false);
    expect(isAccountId(VALID_POOL)).toBe(false);
  });

  it('rejects malformed or wrong-length strings', () => {
    expect(isAccountId('')).toBe(false);
    expect(isAccountId('GDQP2KPQ')).toBe(false); // too short
    expect(isAccountId(VALID_ACCOUNT.slice(0, -1))).toBe(false); // off-by-one
    expect(isAccountId(`${VALID_ACCOUNT}A`)).toBe(false); // too long
  });

  it('rejects characters outside the base32 alphabet', () => {
    expect(isAccountId('G' + '0'.repeat(55))).toBe(false); // 0 not in [A-Z2-7]
    expect(isAccountId('G' + '1'.repeat(55))).toBe(false); // 1 not in [A-Z2-7]
    expect(isAccountId('g' + 'a'.repeat(55))).toBe(false); // lowercase
  });
});

describe('isContractId', () => {
  it('accepts a valid C-strkey', () => {
    expect(isContractId(VALID_CONTRACT)).toBe(true);
  });
  it('rejects accounts, pools, and arbitrary strings', () => {
    expect(isContractId(VALID_ACCOUNT)).toBe(false);
    expect(isContractId(VALID_POOL)).toBe(false);
    expect(isContractId('')).toBe(false);
  });
});

describe('isPoolIdentifier', () => {
  // A Soroban pool IS a contract, so the pool page must accept that form.
  // A stricter rule here rendered "not found" for every Soroban pool the
  // list linked to, without ever reaching the network (task 0374).
  it('accepts both forms a pool can be addressed by', () => {
    expect(isPoolIdentifier(VALID_POOL)).toBe(true);
    expect(isPoolIdentifier(VALID_CONTRACT)).toBe(true);
  });

  it('still rejects what is neither', () => {
    expect(isPoolIdentifier(VALID_ACCOUNT)).toBe(false);
    expect(isPoolIdentifier('')).toBe(false);
  });

  // `isPoolId` stays narrow on purpose: search uses it to tell a pool from a
  // contract, and widening it would route every contract to the pool page.
  it('does not widen isPoolId itself', () => {
    expect(isPoolId(VALID_CONTRACT)).toBe(false);
  });
});

describe('isPoolId', () => {
  it('accepts a valid CAP-38 L-strkey', () => {
    expect(isPoolId(VALID_POOL)).toBe(true);
  });
  it('rejects accounts and contracts', () => {
    expect(isPoolId(VALID_ACCOUNT)).toBe(false);
    expect(isPoolId(VALID_CONTRACT)).toBe(false);
  });
});

describe('isTransactionHash', () => {
  it('accepts a 64-character hex string regardless of case', () => {
    expect(isTransactionHash(VALID_TX)).toBe(true);
    expect(isTransactionHash(VALID_TX.toUpperCase())).toBe(true);
  });
  it('rejects strings that are not 64 hex chars', () => {
    expect(isTransactionHash('')).toBe(false);
    expect(isTransactionHash('abc')).toBe(false);
    expect(isTransactionHash(VALID_TX.slice(0, 63))).toBe(false);
    expect(isTransactionHash(`${VALID_TX}0`)).toBe(false);
    expect(isTransactionHash(`g${VALID_TX.slice(1)}`)).toBe(false); // g is not hex
  });
});

describe('isLedgerSequence', () => {
  it('accepts positive integers (string or number)', () => {
    expect(isLedgerSequence(1)).toBe(true);
    expect(isLedgerSequence('54837201')).toBe(true);
  });
  it('rejects zero, negatives, decimals, and non-numeric strings', () => {
    expect(isLedgerSequence(0)).toBe(false);
    expect(isLedgerSequence('-1')).toBe(false);
    expect(isLedgerSequence('1.5')).toBe(false);
    expect(isLedgerSequence('abc')).toBe(false);
    expect(isLedgerSequence('')).toBe(false);
  });
});

describe('isAssetId', () => {
  it('accepts the reserved native keyword', () => {
    expect(isAssetId('native')).toBe(true);
  });
  it('accepts a contract StrKey (SAC / Soroban asset)', () => {
    expect(isAssetId(VALID_CONTRACT)).toBe(true);
  });
  it('accepts classic CODE-ISSUER', () => {
    expect(isAssetId(`USDC-${VALID_ACCOUNT}`)).toBe(true);
    expect(isAssetId(`A-${VALID_ACCOUNT}`)).toBe(true); // 1-char code
    expect(isAssetId(`USDCUSDCUSDC-${VALID_ACCOUNT}`)).toBe(true); // 12-char
  });
  it('rejects malformed code-issuer and bare strings', () => {
    expect(isAssetId('')).toBe(false);
    expect(isAssetId('USDC')).toBe(false); // no issuer
    expect(isAssetId(`-${VALID_ACCOUNT}`)).toBe(false); // empty code
    expect(isAssetId(`USDC-${VALID_CONTRACT}`)).toBe(false); // issuer not G
    expect(isAssetId(`USDCUSDCUSDCX-${VALID_ACCOUNT}`)).toBe(false); // 13-char code
    expect(isAssetId('42')).toBe(false); // numeric surrogate is NOT a canonical id
  });
});
