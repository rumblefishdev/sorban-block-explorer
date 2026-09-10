const HEX_64 = /^[0-9a-fA-F]{64}$/;
const STELLAR_ACCOUNT = /^G[A-Z2-7]{55}$/;
const STELLAR_CONTRACT = /^C[A-Z2-7]{55}$/;
const STELLAR_POOL = /^L[A-Z2-7]{55}$/;
const POSITIVE_INT = /^\d+$/;
/** Classic asset code — 1-12 alphanumeric chars (SEP-11 `AssetCode`). */
const ASSET_CODE = /^[A-Za-z0-9]{1,12}$/;

export function isTransactionHash(value: string): boolean {
  return HEX_64.test(value);
}

export function isAccountId(value: string): boolean {
  return STELLAR_ACCOUNT.test(value);
}

export function isContractId(value: string): boolean {
  return STELLAR_CONTRACT.test(value);
}

export function isLedgerSequence(value: string | number): boolean {
  const s = String(value);
  return POSITIVE_INT.test(s) && Number(s) > 0;
}

export function isPoolId(value: string): boolean {
  return STELLAR_POOL.test(value);
}

/**
 * Either form a POOL can be addressed by.
 *
 * A classic pool is a ledger entry with a CAP-38 / SEP-23 `L…` strkey; a
 * Soroban pool IS a contract, so it is addressed by its `C…` address. The two
 * are distinct types elsewhere — search must not send every contract to the
 * pool page — which is why this is its own rule rather than a widened
 * {@link isPoolId}.
 *
 * Mirrors the API, whose pool routes accept both (task 0374). They must agree:
 * a stricter check here turns a live pool into a "not found" that never even
 * reaches the network, which is exactly what it did for every Soroban pool the
 * list itself linked to.
 */
export function isPoolIdentifier(value: string): boolean {
  return isPoolId(value) || isContractId(value);
}

/**
 * Canonical asset-id token (polymorphic). Accepts the three forms the
 * `/assets/:id` route serves: the reserved `native` keyword, a contract
 * StrKey (a SAC / Soroban asset), or classic `CODE-ISSUER` (a 1-12 char code
 * and a G-strkey issuer). The numeric surrogate row-id is a backend internal,
 * not a canonical FE asset id, so it is intentionally rejected here.
 */
export function isAssetId(value: string): boolean {
  if (value === 'native') return true;
  if (STELLAR_CONTRACT.test(value)) return true;
  const dash = value.indexOf('-');
  if (dash <= 0) return false;
  const code = value.slice(0, dash);
  const issuer = value.slice(dash + 1);
  return ASSET_CODE.test(code) && STELLAR_ACCOUNT.test(issuer);
}
