import type { ChipProps } from '@rumblefish/soroban-block-explorer-ui';
import {
  DEFAULT_TRUNCATION,
  NATIVE_ASSET_CODE,
  truncateMiddle,
} from '@rumblefish/soroban-block-explorer-ui';

export interface AssetTypeMeta {
  /** Human-readable badge label. */
  label: string;
  /** Chip colour distinguishing the asset class. */
  color: ChipProps['color'];
}

/**
 * Maps the API `asset_type_name` (`native | classic_credit | soroban`) to a
 * TYPE badge label and colour. Asset identity is the most confusing area for
 * users, so each class gets a visually distinct chip. This axis is orthogonal
 * to the SAC facet: `sac` is NOT an `asset_type_name` (ADR 0051 — a SAC is a
 * facet of a classic_credit / native row), so it is surfaced as a separate
 * {@link SAC_TAG} property tag, never as a type here.
 */
const META: Record<string, AssetTypeMeta> = {
  native: { label: 'Native', color: 'blue' },
  classic_credit: { label: 'Classic credit', color: 'neutral' },
  soroban: { label: 'Soroban', color: 'emerald' },
};

export function assetTypeMeta(typeName?: string | null): AssetTypeMeta {
  const meta = typeName ? META[typeName] : undefined;
  return meta ?? { label: typeName ?? 'Unknown', color: 'neutral' };
}

// The native-XLM vocabulary lives in libs/ui/identifiers (task 0472) — the
// format layer down there needs it too, so it cannot live up here. Re-exported
// for the many page-level callers that reach for it alongside the asset-chip
// metadata in this file.
export {
  NATIVE_ASSET_CODE,
  isNativeAssetString,
} from '@rumblefish/soroban-block-explorer-ui';

/**
 * The one ladder that names an asset — title, breadcrumb, table cell, avatar
 * letter, pool leg, balance-change row. Each rung is the only thing that names
 * an asset of that kind, so the order is a fact about the ledger, not a
 * preference:
 *
 *   1. **native** → `XLM`. Native carries no `asset_code` on the ledger, so the
 *      TYPE names it. This is the single native rule in the naming path: the
 *      pool legs, the asset pages and the balance-change rows each used to
 *      carry their own copy of it.
 *   2. **`asset_code`** → the classic code.
 *   3. **`symbol`** → the on-chain SEP-41 symbol, for a soroban token with no
 *      classic code (task 0304).
 *   4. **`contract_id`** → the truncated contract address. A soroban token can
 *      publish no symbol at all, and its contract IS its identity — showing
 *      `CAQC…RZQB` names it honestly, where a dash claims the row is empty.
 *      Truncated with the app-wide standard so it reads like every other
 *      address reference.
 *
 * Returns `null` only when NOTHING identifies the asset — schema drift, not a
 * nameless token. Callers pick their own empty rendering for it.
 *
 * An empty-string code counts as absent: the ledger writes native's code that
 * way, and no other asset has one.
 */
export function assetDisplayCode(asset: {
  asset_type_name?: string | null;
  asset_code?: string | null;
  symbol?: string | null;
  contract_id?: string | null;
}): string | null {
  if (asset.asset_type_name === 'native') return NATIVE_ASSET_CODE;
  if (asset.asset_code) return asset.asset_code;
  if (asset.symbol) return asset.symbol;
  if (asset.contract_id) {
    return truncateMiddle(asset.contract_id, DEFAULT_TRUNCATION);
  }
  return null;
}

/**
 * The "SAC" property tag (ADR 0051), rendered IN ADDITION to the type badge on
 * an asset that carries a DEPLOYED Stellar Asset Contract facet (`sac_deployed`).
 * A reserved (un-deployed) SAC address gets no tag — it is not a live contract.
 */
export const SAC_TAG: AssetTypeMeta = { label: 'SAC', color: 'brown' };

/**
 * Type-filter options for the assets list (the type-chip row). "SAC" is NOT here
 * — it is a separate "Has SAC" PROPERTY toggle (ADR 0051) mapped by the list
 * page to `filter[sac]=true`, orthogonal to the asset type.
 */
export const ASSET_TYPE_FILTERS: readonly { label: string; value: string }[] = [
  { label: 'All types', value: '' },
  { label: 'Classic credit', value: 'classic_credit' },
  { label: 'Soroban', value: 'soroban' },
];
