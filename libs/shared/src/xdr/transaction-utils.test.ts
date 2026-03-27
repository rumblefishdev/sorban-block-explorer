import { describe, it, expect } from 'vitest';
import {
  xdr,
  Keypair,
  Networks,
  TransactionBuilder,
  Account,
  Operation,
  Asset,
  Memo,
} from '@stellar/stellar-base';
import { computeTransactionHash, extractMemo } from './transaction-utils.js';

function buildEnvelope(memo?: Memo): { envelopeXdr: string; hash: string } {
  const keypair = Keypair.random();
  const account = new Account(keypair.publicKey(), '100');
  const builder = new TransactionBuilder(account, {
    fee: '100',
    networkPassphrase: Networks.TESTNET,
  });

  if (memo) {
    builder.addMemo(memo);
  }

  const tx = builder
    .addOperation(
      Operation.payment({
        destination: Keypair.random().publicKey(),
        asset: Asset.native(),
        amount: '10',
      })
    )
    .setTimeout(30)
    .build();

  return {
    envelopeXdr: tx.toEnvelope().toXDR('base64'),
    hash: tx.hash().toString('hex'),
  };
}

describe('computeTransactionHash', () => {
  it('produces a 64-character hex string', () => {
    const { envelopeXdr } = buildEnvelope();
    const hash = computeTransactionHash(envelopeXdr, Networks.TESTNET);
    expect(hash).toMatch(/^[0-9a-f]{64}$/);
  });

  it('matches the SDK-computed hash', () => {
    const { envelopeXdr, hash: expected } = buildEnvelope();
    const hash = computeTransactionHash(envelopeXdr, Networks.TESTNET);
    expect(hash).toBe(expected);
  });
});

describe('extractMemo', () => {
  it('returns none for no memo', () => {
    const { envelopeXdr } = buildEnvelope();
    const envelope = xdr.TransactionEnvelope.fromXDR(envelopeXdr, 'base64');
    expect(extractMemo(envelope)).toEqual({ memoType: 'none', memo: null });
  });

  it('extracts text memo', () => {
    const { envelopeXdr } = buildEnvelope(Memo.text('hello'));
    const envelope = xdr.TransactionEnvelope.fromXDR(envelopeXdr, 'base64');
    expect(extractMemo(envelope)).toEqual({ memoType: 'text', memo: 'hello' });
  });

  it('extracts id memo', () => {
    const { envelopeXdr } = buildEnvelope(Memo.id('12345'));
    const envelope = xdr.TransactionEnvelope.fromXDR(envelopeXdr, 'base64');
    const result = extractMemo(envelope);
    expect(result.memoType).toBe('id');
    expect(result.memo).toBe('12345');
  });

  it('extracts hash memo', () => {
    const hashBuf = Buffer.alloc(32, 0xab);
    const { envelopeXdr } = buildEnvelope(Memo.hash(hashBuf.toString('hex')));
    const envelope = xdr.TransactionEnvelope.fromXDR(envelopeXdr, 'base64');
    const result = extractMemo(envelope);
    expect(result.memoType).toBe('hash');
    expect(result.memo).toBe(hashBuf.toString('hex'));
  });

  it('extracts return memo', () => {
    const retHash = Buffer.alloc(32, 0xcd);
    const { envelopeXdr } = buildEnvelope(Memo.return(retHash.toString('hex')));
    const envelope = xdr.TransactionEnvelope.fromXDR(envelopeXdr, 'base64');
    const result = extractMemo(envelope);
    expect(result.memoType).toBe('return');
    expect(result.memo).toBe(retHash.toString('hex'));
  });
});
