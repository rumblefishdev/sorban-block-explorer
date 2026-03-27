import { describe, it, expect } from 'vitest';
import { xdr, Keypair, StrKey } from '@stellar/stellar-base';
import {
  extractContractDeployments,
  extractAccountStates,
  extractLiquidityPoolStates,
} from './ledger-entry-extractors.js';

// The SDK types use Opaque[] for Hash/ContractId but runtime is Buffer.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const asHash = (buf: Buffer): any => buf;

function makeCreatedChange(data: xdr.LedgerEntryData): xdr.LedgerEntryChange {
  return xdr.LedgerEntryChange.ledgerEntryCreated(
    new xdr.LedgerEntry({
      lastModifiedLedgerSeq: 100,
      data,
      ext: new xdr.LedgerEntryExt(0),
    })
  );
}

function makeUpdatedChange(data: xdr.LedgerEntryData): xdr.LedgerEntryChange {
  return xdr.LedgerEntryChange.ledgerEntryUpdated(
    new xdr.LedgerEntry({
      lastModifiedLedgerSeq: 101,
      data,
      ext: new xdr.LedgerEntryExt(0),
    })
  );
}

describe('extractContractDeployments', () => {
  it('extracts a WASM contract deployment', () => {
    const wasmHash = Buffer.alloc(32, 0xaa);
    const contractAddress = xdr.ScAddress.scAddressTypeContract(
      asHash(Buffer.alloc(32, 0xbb))
    );

    const contractData = xdr.LedgerEntryData.contractData(
      new xdr.ContractDataEntry({
        ext: xdr.ExtensionPoint.fromXDR(Buffer.alloc(4, 0)),
        contract: contractAddress,
        key: xdr.ScVal.scvLedgerKeyContractInstance(),
        durability: xdr.ContractDataDurability.persistent(),
        val: xdr.ScVal.scvContractInstance(
          new xdr.ScContractInstance({
            executable: xdr.ContractExecutable.contractExecutableWasm(
              asHash(wasmHash)
            ),
            storage: null,
          })
        ),
      })
    );

    const changes = [makeCreatedChange(contractData)];
    const results = extractContractDeployments(changes);

    expect(results).toHaveLength(1);
    expect(results[0]?.contractId).toMatch(/^C[A-Z0-9]+$/);
    expect(results[0]?.wasmHash).toBe(wasmHash.toString('hex'));
    expect(results[0]?.isSac).toBe(false);
  });

  it('extracts a SAC deployment', () => {
    const contractAddress = xdr.ScAddress.scAddressTypeContract(
      asHash(Buffer.alloc(32, 0xcc))
    );

    const contractData = xdr.LedgerEntryData.contractData(
      new xdr.ContractDataEntry({
        ext: xdr.ExtensionPoint.fromXDR(Buffer.alloc(4, 0)),
        contract: contractAddress,
        key: xdr.ScVal.scvLedgerKeyContractInstance(),
        durability: xdr.ContractDataDurability.persistent(),
        val: xdr.ScVal.scvContractInstance(
          new xdr.ScContractInstance({
            executable: xdr.ContractExecutable.contractExecutableStellarAsset(),
            storage: null,
          })
        ),
      })
    );

    const changes = [makeCreatedChange(contractData)];
    const results = extractContractDeployments(changes);

    expect(results).toHaveLength(1);
    expect(results[0]?.wasmHash).toBeNull();
    expect(results[0]?.isSac).toBe(true);
  });

  it('ignores non-created entries', () => {
    const contractAddress = xdr.ScAddress.scAddressTypeContract(
      asHash(Buffer.alloc(32, 0xdd))
    );

    const contractData = xdr.LedgerEntryData.contractData(
      new xdr.ContractDataEntry({
        ext: xdr.ExtensionPoint.fromXDR(Buffer.alloc(4, 0)),
        contract: contractAddress,
        key: xdr.ScVal.scvLedgerKeyContractInstance(),
        durability: xdr.ContractDataDurability.persistent(),
        val: xdr.ScVal.scvContractInstance(
          new xdr.ScContractInstance({
            executable: xdr.ContractExecutable.contractExecutableStellarAsset(),
            storage: null,
          })
        ),
      })
    );

    const changes = [makeUpdatedChange(contractData)];
    expect(extractContractDeployments(changes)).toHaveLength(0);
  });

  it('returns empty for no contract entries', () => {
    expect(extractContractDeployments([])).toHaveLength(0);
  });
});

describe('extractAccountStates', () => {
  it('extracts account state with strkey accountId', () => {
    const keypair = Keypair.random();
    const accountEntry = xdr.LedgerEntryData.account(
      new xdr.AccountEntry({
        accountId: xdr.PublicKey.publicKeyTypeEd25519(keypair.rawPublicKey()),
        balance: new xdr.Int64(1000000),
        seqNum: new xdr.Int64(42),
        numSubEntries: 0,
        inflationDest: null,
        flags: 0,
        homeDomain: 'example.com',
        thresholds: Buffer.from([1, 0, 0, 0]),
        signers: [],
        ext: new xdr.AccountEntryExt(0),
      })
    );

    const changes = [makeCreatedChange(accountEntry)];
    const results = extractAccountStates(changes);

    expect(results).toHaveLength(1);
    expect(results[0]?.accountId).toBe(keypair.publicKey());
    expect(results[0]?.sequenceNumber).toBe('42');
    expect(results[0]?.homeDomain).toBe('example.com');
  });

  it('returns null homeDomain when empty', () => {
    const keypair = Keypair.random();
    const accountEntry = xdr.LedgerEntryData.account(
      new xdr.AccountEntry({
        accountId: xdr.PublicKey.publicKeyTypeEd25519(keypair.rawPublicKey()),
        balance: new xdr.Int64(0),
        seqNum: new xdr.Int64(1),
        numSubEntries: 0,
        inflationDest: null,
        flags: 0,
        homeDomain: '',
        thresholds: Buffer.from([1, 0, 0, 0]),
        signers: [],
        ext: new xdr.AccountEntryExt(0),
      })
    );

    const changes = [makeUpdatedChange(accountEntry)];
    const results = extractAccountStates(changes);

    expect(results).toHaveLength(1);
    expect(results[0]?.homeDomain).toBeNull();
  });
});

describe('extractLiquidityPoolStates', () => {
  it('returns empty for no pool entries', () => {
    expect(extractLiquidityPoolStates([])).toHaveLength(0);
  });

  it('extracts constant product pool state', () => {
    const issuerKeypair = Keypair.random();

    const poolEntry = xdr.LedgerEntryData.liquidityPool(
      new xdr.LiquidityPoolEntry({
        liquidityPoolId: asHash(Buffer.alloc(32, 0xee)),
        body: xdr.LiquidityPoolEntryBody.liquidityPoolConstantProduct(
          new xdr.LiquidityPoolEntryConstantProduct({
            params: new xdr.LiquidityPoolConstantProductParameters({
              assetA: xdr.Asset.assetTypeNative(),
              assetB: xdr.Asset.assetTypeCreditAlphanum4(
                new xdr.AlphaNum4({
                  assetCode: Buffer.from('USDC\0\0\0\0'),
                  issuer: xdr.PublicKey.publicKeyTypeEd25519(
                    issuerKeypair.rawPublicKey()
                  ),
                })
              ),
              fee: 30,
            }),
            reserveA: new xdr.Int64(5000000),
            reserveB: new xdr.Int64(10000000),
            totalPoolShares: new xdr.Int64(7000000),
            poolSharesTrustLineCount: new xdr.Int64(5),
          })
        ),
      })
    );

    const changes = [makeCreatedChange(poolEntry)];
    const results = extractLiquidityPoolStates(changes);

    expect(results).toHaveLength(1);
    expect(results[0]?.assetA).toBe('native');
    expect(results[0]?.assetB).toMatch(/^USDC:G[A-Z0-9]+$/);
    expect(results[0]?.reserveA).toBe('5000000');
    expect(results[0]?.reserveB).toBe('10000000');
    expect(results[0]?.totalShares).toBe('7000000');

    const issuerPart = results[0]?.assetB.split(':')[1];
    expect(StrKey.isValidEd25519PublicKey(issuerPart ?? '')).toBe(true);
  });
});
