import type { PoolAssetLeg, PoolItem } from '@rumblefish/api-types';

import { assetColor } from '../assets/assetColor.js';
import { assetDisplayCode } from '../assets/assetType.js';
import { routes } from '../../router/routes.js';

const SEVEN_DAYS_MS = 7 * 24 * 60 * 60 * 1000;

/**
 * Resolve the cross-entity link target for a pool asset leg (task 0263).
 * Always routes to the asset detail page — backend `parse_asset_id`
 * accepts either the SAC C-strkey or a `code-issuer` composite, so both
 * classic and SAC legs resolve to the same asset row.
 *
 * Precedence:
 *   1. `asset_type_name === 'native'` (native XLM) → `/assets/native`. The reserved
 *      `native` literal IS the canonical asset token (task 0243) — the older
 *      "native has no on-chain address, so no link" rule predates it and left
 *      XLM as the only unlinkable leg in the app, while account balances,
 *      search and the SAC chip all route there (task 0472).
 *   2. `asset_code` + `issuer` (classic credit) → `/assets/${code}-${issuer}`.
 *      Preferred over the SAC C-address ON PURPOSE: task 0364 dropped the
 *      SAC-facet aliasing arm from `fetch_by_contract_id` (asset_type is
 *      pinned to 3), so `/assets/{SAC C…}` 404s — verified against the API
 *      2026-08-13. The earlier contract_id-first order sent ~93k classic
 *      legs to a dead page.
 *   3. `contract_id` → `/assets/${contract_id}`. Only reachable when the
 *      code/issuer pair is incomplete; resolves for a genuine Soroban token
 *      contract, 404s for a bare SAC address — a best-effort last resort.
 *   4. Anything else (schema drift) → no link.
 */
export function legHref(leg: PoolAssetLeg): string | undefined {
  if (leg.asset_type_name === 'native') return routes.asset('native');
  if (leg.asset_code && leg.issuer) {
    return routes.asset(`${leg.asset_code}-${leg.issuer}`);
  }
  if (leg.contract_id) return routes.asset(leg.contract_id);
  return undefined;
}

/**
 * The display label for one leg — the app-wide {@link assetDisplayCode} ladder,
 * narrowed to the guarantee a leg carries: SOMETHING always identifies it.
 * Native is named by its type, a classic leg by its code, a soroban leg by its
 * symbol or, failing that, by its own contract address.
 *
 * **Hard-fail on schema drift.** A leg that hits none of those rungs is a
 * broken backend contract, not a nameless token — throw rather than render a
 * `?` placeholder, so the surrounding `SectionErrorBoundary` catches it instead
 * of it leaking into the UI. The throw used to fire for any leg without an
 * `asset_code`, which a soroban token legitimately has none of; it now fires
 * only when nothing at all names the leg.
 */
export function assetLegLabel(leg: PoolAssetLeg): string {
  const label = assetDisplayCode(leg);
  if (label != null) return label;
  throw new Error(
    `assetLegLabel: nothing identifies this leg (asset_type_name=${
      leg.asset_type_name ?? 'null'
    })`
  );
}

/**
 * Shown for a pool whose legs are not in the index yet. An empty name would
 * read as a pool that holds nothing — a plausible-looking wrong answer, which
 * is worse than saying the data is missing.
 */
export const UNINDEXED_POOL_LABEL = 'Composition not indexed';

/**
 * The pool's name — its legs' labels, in registration order. Two for a classic
 * pool, up to four for a soroban one, so the separator repeats rather than
 * joining a fixed left and right.
 */
export function poolLabel(legs: readonly PoolAssetLeg[]): string {
  if (legs.length === 0) return UNINDEXED_POOL_LABEL;
  return legs.map(assetLegLabel).join(' / ');
}

/**
 * Each leg paired with the reserve it holds.
 *
 * The snapshot the amounts come from is still pair-shaped (`reserve_a` /
 * `reserve_b`), so only the first two legs can carry one — a soroban pool's
 * third and fourth reserves are not indexed yet. Such a leg is listed with
 * `undefined` rather than dropped, so the pool's composition still reads in
 * full and the missing amount renders as the same "—" a stale pool shows,
 * instead of the leg silently disappearing.
 */
export function poolReserves(
  pool: Pick<PoolItem, 'legs' | 'reserve_a' | 'reserve_b'>
): { leg: PoolAssetLeg; amount: string | null }[] {
  const amounts = [pool.reserve_a, pool.reserve_b];
  // `null`, not `undefined`: every consumer renders "no amount" the same way,
  // so a third state would only be visible to a test.
  return pool.legs.map((leg, i) => ({ leg, amount: amounts[i] ?? null }));
}

/**
 * Reserve-dot colour for a pool leg — the saturated mid-tone of the leg's
 * per-asset colour (`assetColor`), keyed identically to the leg avatar so
 * each reserve row's dot matches its asset's avatar.
 */
export function reserveDotColor(leg: PoolAssetLeg): string {
  return assetColor(assetLegLabel(leg)).dot;
}
/**
 * A pool is "stale" when its newest snapshot is older than 7 days (matches
 * the freshness window enforced by `18_get_liquidity_pools_list.sql` and
 * the participants endpoint). Stale pools come back with `null` reserves,
 * TVL, volume, and fee revenue. `participant_count` stays accurate
 * regardless of freshness (per 0246).
 */
export function isPoolStale(
  latestSnapshotAt: string | null | undefined
): boolean {
  if (!latestSnapshotAt) return true;
  const ageMs = Date.now() - new Date(latestSnapshotAt).getTime();
  return Number.isNaN(ageMs) || ageMs > SEVEN_DAYS_MS;
}
