import type { PoolAssetLeg } from '@rumblefish/api-types';
import { formatCompactAmount } from '@rumblefish/soroban-block-explorer-ui';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
  assetLegLabel,
  isPoolStale,
  legHref,
  poolLabel,
  poolReserves,
} from './helpers.js';

function makeLeg(overrides: Partial<PoolAssetLeg> = {}): PoolAssetLeg {
  return {
    asset_code: 'USDC',
    asset_type_name: 'classic_credit',
    contract_id: null,
    issuer: 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN',
    icon_url: null,
    sac_contract_id: null,
    ...overrides,
  };
}

describe('legHref', () => {
  // Changed in task 0472 — this case previously asserted `undefined`. Not a
  // regression: `/assets/native` became the canonical asset token in 0243,
  // which retired the "native has no address" rationale this rule was built
  // on. XLM was the only leg in the app that rendered as dead text.
  it('links native legs to the canonical /assets/native token', () => {
    expect(legHref(makeLeg({ asset_type_name: 'native' }))).toBe(
      '/assets/native'
    );
  });

  it('prefers the canonical native token over an XLM SAC mirror', () => {
    expect(
      legHref(
        makeLeg({
          asset_type_name: 'native',
          asset_code: null,
          issuer: null,
          contract_id:
            'CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA',
        })
      )
    ).toBe('/assets/native');
  });

  // Flipped in task 0472 — this case previously asserted contract_id-first.
  // Intentional: task 0364 dropped SAC-facet aliasing from the assets
  // endpoint, so /assets/{SAC C…} 404s and the pair is the only live route
  // for a classic leg (~93k legs carry a SAC mirror on prod).
  it('prefers code-issuer over the SAC mirror, whose address now 404s', () => {
    expect(
      legHref(
        makeLeg({
          contract_id:
            'CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC',
        })
      )
    ).toBe(
      `/assets/${encodeURIComponent(
        'USDC-GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
      )}`
    );
  });

  it('falls back to contract_id only when the pair is incomplete', () => {
    expect(
      legHref(
        makeLeg({
          asset_code: null,
          issuer: null,
          asset_type_name: 'soroban',
          contract_id:
            'CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC',
        })
      )
    ).toBe('/assets/CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC');
  });

  it('falls back to /assets/{code}-{issuer} for classic credit legs', () => {
    const href = legHref(makeLeg());
    expect(href).toBe(
      `/assets/${encodeURIComponent(
        'USDC-GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
      )}`
    );
  });

  it('returns undefined for schema-drift legs (no code, no contract id)', () => {
    expect(
      legHref(makeLeg({ asset_code: null, issuer: null, contract_id: null }))
    ).toBeUndefined();
  });
});

describe('assetLegLabel', () => {
  it('returns "XLM" for the native leg', () => {
    expect(
      assetLegLabel(makeLeg({ asset_type_name: 'native', asset_code: null }))
    ).toBe('XLM');
  });

  it('returns the asset_code for non-native legs', () => {
    expect(assetLegLabel(makeLeg({ asset_code: 'USDC' }))).toBe('USDC');
    expect(assetLegLabel(makeLeg({ asset_code: 'EURC' }))).toBe('EURC');
  });

  // The case that made this worth changing: a soroban token publishes no
  // classic code, and this used to throw for every one of them — which would
  // have taken down the whole list the moment soroban pools appeared in it.
  it('names a code-less soroban leg by its truncated contract address', () => {
    expect(
      assetLegLabel(
        makeLeg({
          asset_type_name: 'soroban',
          asset_code: null,
          issuer: null,
          contract_id:
            'CAQCFVLOBK5GIULPNZRGSXFPMIDUTBDDKCEHQNCZGYNK5JEN6IY5RZQB',
        })
      )
    ).toBe('CAQC…RZQB');
  });

  it('still throws when NOTHING identifies the leg', () => {
    expect(() =>
      assetLegLabel(
        makeLeg({
          asset_code: null,
          issuer: null,
          contract_id: null,
          asset_type_name: 'classic_credit',
        })
      )
    ).toThrow(/nothing identifies/);
  });
});

describe('poolLabel', () => {
  // A pool whose legs are not backfilled yet must not read as a pool that
  // holds nothing — an empty name is a plausible-looking wrong answer.
  it('says so when the legs are not indexed, rather than rendering blank', () => {
    expect(poolLabel([])).toBe('Composition not indexed');
  });

  it('joins every leg, not just a left and a right', () => {
    expect(
      poolLabel([
        makeLeg({ asset_type_name: 'native', asset_code: null }),
        makeLeg({ asset_code: 'USDC' }),
        makeLeg({ asset_code: 'EURC' }),
      ])
    ).toBe('XLM / USDC / EURC');
  });
});

describe('poolReserves', () => {
  it('reads the amount off the leg that holds it', () => {
    const legs = [
      makeLeg({
        asset_type_name: 'native',
        asset_code: null,
        reserve: '100.0',
      }),
      makeLeg({ asset_code: 'USDC', reserve: '25.0' }),
    ];
    expect(poolReserves({ legs })).toEqual([
      { leg: legs[0], amount: '100.0' },
      { leg: legs[1], amount: '25.0' },
    ]);
  });

  // Three amounts for three legs — the shape a `reserve_a` / `reserve_b` pair
  // could never hold, and the reason the value moved onto the leg.
  it('carries an amount for every leg, not just two', () => {
    const legs = [
      makeLeg({ asset_code: 'USDC', reserve: '1' }),
      makeLeg({ asset_code: 'EURC', reserve: '2' }),
      makeLeg({ asset_code: 'DAI', reserve: '3' }),
    ];
    expect(poolReserves({ legs }).map((r) => r.amount)).toEqual([
      '1',
      '2',
      '3',
    ]);
  });

  it('lists a leg whose amount no source knows, rather than dropping it', () => {
    const legs = [
      makeLeg({ asset_code: 'USDC', reserve: '1' }),
      makeLeg({ asset_code: 'EURC', reserve: null }),
    ];
    const rows = poolReserves({ legs });
    expect(rows).toHaveLength(2);
    expect(rows[1]).toEqual({ leg: legs[1], amount: null });
  });
});

describe('isPoolStale', () => {
  beforeEach(() => {
    // Pin "now" so the freshness math is deterministic.
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-05-29T12:00:00Z'));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('returns true when no snapshot timestamp is provided', () => {
    expect(isPoolStale(null)).toBe(true);
    expect(isPoolStale(undefined)).toBe(true);
    expect(isPoolStale('')).toBe(true);
  });

  it('returns false within the 7-day freshness window', () => {
    expect(isPoolStale('2026-05-29T11:00:00Z')).toBe(false); // 1h ago
    expect(isPoolStale('2026-05-23T13:00:00Z')).toBe(false); // ~6 days ago
  });

  it('returns true once the snapshot is older than 7 days', () => {
    expect(isPoolStale('2026-05-22T11:59:00Z')).toBe(true);
    expect(isPoolStale('2026-05-01T00:00:00Z')).toBe(true);
  });

  it('returns true on unparseable timestamps', () => {
    expect(isPoolStale('not a date')).toBe(true);
  });
});

describe('formatCompactAmount', () => {
  it('returns em-dash for null, undefined, and non-numeric input', () => {
    expect(formatCompactAmount(null)).toBe('—');
    expect(formatCompactAmount(undefined)).toBe('—');
    expect(formatCompactAmount('not a number')).toBe('—');
  });

  it('formats small numbers without notation', () => {
    expect(formatCompactAmount(0)).toBe('0');
    expect(formatCompactAmount(42)).toBe('42');
  });

  it('uses compact notation for larger numbers', () => {
    expect(formatCompactAmount(1_500)).toBe('1.5K');
    expect(formatCompactAmount(1_200_000)).toBe('1.2M');
    expect(formatCompactAmount('753982100.00')).toBe('754M');
  });
});
