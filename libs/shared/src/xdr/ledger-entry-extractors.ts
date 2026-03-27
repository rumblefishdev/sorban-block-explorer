import { xdr, Address, StrKey } from '@stellar/stellar-base';

// --- Extracted record types ---

export interface ExtractedContractDeployment {
  contractId: string;
  wasmHash: string | null;
  deployerAccount: string | null;
  isSac: boolean;
}

export interface ExtractedAccountState {
  accountId: string;
  sequenceNumber: string;
  homeDomain: string | null;
}

export interface ExtractedLiquidityPoolState {
  poolId: string;
  assetA: string;
  assetB: string;
  reserveA: string;
  reserveB: string;
  totalShares: string;
}

// --- Extractors ---

function entryDataFromChange(
  change: xdr.LedgerEntryChange
): xdr.LedgerEntryData | null {
  const type = change.switch();
  if (
    type.value === xdr.LedgerEntryChangeType.ledgerEntryCreated().value ||
    type.value === xdr.LedgerEntryChangeType.ledgerEntryUpdated().value ||
    type.value === xdr.LedgerEntryChangeType.ledgerEntryState().value ||
    type.value === xdr.LedgerEntryChangeType.ledgerEntryRestored().value
  ) {
    return (change.value() as xdr.LedgerEntry).data();
  }
  return null;
}

function formatAsset(asset: xdr.Asset): string {
  const type = asset.switch();
  if (type.value === xdr.AssetType.assetTypeNative().value) {
    return 'native';
  }
  if (type.value === xdr.AssetType.assetTypeCreditAlphanum4().value) {
    const a4 = asset.alphaNum4();
    const code = a4.assetCode().toString('utf8').replace(/\0/g, '');
    const issuer = StrKey.encodeEd25519PublicKey(
      a4.issuer().ed25519() as unknown as Buffer
    );
    return `${code}:${issuer}`;
  }
  if (type.value === xdr.AssetType.assetTypeCreditAlphanum12().value) {
    const a12 = asset.alphaNum12();
    const code = a12.assetCode().toString('utf8').replace(/\0/g, '');
    const issuer = StrKey.encodeEd25519PublicKey(
      a12.issuer().ed25519() as unknown as Buffer
    );
    return `${code}:${issuer}`;
  }
  return 'unknown';
}

/**
 * Extract contract deployment info from LedgerEntryChanges.
 * Looks for ContractData entries with the LedgerKeyContractInstance key.
 */
export function extractContractDeployments(
  changes: xdr.LedgerEntryChange[]
): ExtractedContractDeployment[] {
  const results: ExtractedContractDeployment[] = [];

  for (const change of changes) {
    // Only process created entries for new deployments
    if (
      change.switch().value !==
      xdr.LedgerEntryChangeType.ledgerEntryCreated().value
    ) {
      continue;
    }

    const data = entryDataFromChange(change);
    if (!data) continue;
    if (data.switch().value !== xdr.LedgerEntryType.contractData().value) {
      continue;
    }

    const contractData = data.contractData();
    const key = contractData.key();

    // The contract instance entry has key = ScVal::LedgerKeyContractInstance
    if (
      key.switch().value !== xdr.ScValType.scvLedgerKeyContractInstance().value
    ) {
      continue;
    }

    const contractAddress = contractData.contract();
    const contractId = Address.fromScAddress(contractAddress).toString();

    const val = contractData.val();
    let wasmHash: string | null = null;
    let isSac = false;

    if (val.switch().value === xdr.ScValType.scvContractInstance().value) {
      const instance = val.instance();
      const exec = instance.executable();
      if (
        exec.switch().value ===
        xdr.ContractExecutableType.contractExecutableWasm().value
      ) {
        wasmHash = exec.wasmHash().toString('hex');
      } else {
        isSac = true;
      }
    }

    results.push({ contractId, wasmHash, deployerAccount: null, isSac });
  }

  return results;
}

/**
 * Extract account state from LedgerEntryChanges.
 */
export function extractAccountStates(
  changes: xdr.LedgerEntryChange[]
): ExtractedAccountState[] {
  const results: ExtractedAccountState[] = [];

  for (const change of changes) {
    const data = entryDataFromChange(change);
    if (!data) continue;
    if (data.switch().value !== xdr.LedgerEntryType.account().value) {
      continue;
    }

    const account = data.account();
    const accountId = StrKey.encodeEd25519PublicKey(
      account.accountId().ed25519() as unknown as Buffer
    );
    const seqNum = account.seqNum().toString();
    const homeDomainBuf = account.homeDomain();
    const homeDomain =
      homeDomainBuf && homeDomainBuf.length > 0
        ? typeof homeDomainBuf === 'string'
          ? homeDomainBuf
          : homeDomainBuf.toString('utf8')
        : null;

    results.push({
      accountId,
      sequenceNumber: seqNum,
      homeDomain,
    });
  }

  return results;
}

/**
 * Extract liquidity pool state from LedgerEntryChanges.
 */
export function extractLiquidityPoolStates(
  changes: xdr.LedgerEntryChange[]
): ExtractedLiquidityPoolState[] {
  const results: ExtractedLiquidityPoolState[] = [];

  for (const change of changes) {
    const data = entryDataFromChange(change);
    if (!data) continue;
    if (data.switch().value !== xdr.LedgerEntryType.liquidityPool().value) {
      continue;
    }

    const pool = data.liquidityPool();
    const poolId = Buffer.from(
      pool.liquidityPoolId() as unknown as Buffer
    ).toString('hex');
    const body = pool.body();

    // Currently only constant product pools exist
    const cp = body.constantProduct();
    const params = cp.params();
    const assetA = formatAsset(params.assetA());
    const assetB = formatAsset(params.assetB());
    const reserveA = cp.reserveA().toString();
    const reserveB = cp.reserveB().toString();
    const totalShares = cp.totalPoolShares().toString();

    results.push({ poolId, assetA, assetB, reserveA, reserveB, totalShares });
  }

  return results;
}
