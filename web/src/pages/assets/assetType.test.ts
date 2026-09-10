import { describe, expect, it } from 'vitest';

import {
  ASSET_TYPE_FILTERS,
  assetDisplayCode,
  assetTypeMeta,
  SAC_TAG,
} from './assetType.js';

describe('assetTypeMeta', () => {
  it('maps known type names to label + color', () => {
    expect(assetTypeMeta('native')).toEqual({ label: 'Native', color: 'blue' });
    expect(assetTypeMeta('classic_credit')).toEqual({
      label: 'Classic credit',
      color: 'neutral',
    });
    expect(assetTypeMeta('soroban')).toEqual({
      label: 'Soroban',
      color: 'emerald',
    });
  });

  it('does not treat "sac" as a type — SAC is a separate property tag (ADR 0051)', () => {
    // `sac` is no longer an `asset_type_name`; it falls back to the raw label.
    expect(assetTypeMeta('sac')).toEqual({ label: 'sac', color: 'neutral' });
  });

  it('falls back to the raw type as label with neutral color when unknown', () => {
    expect(assetTypeMeta('mystery')).toEqual({
      label: 'mystery',
      color: 'neutral',
    });
  });

  it('falls back to "Unknown" when type is null/undefined', () => {
    expect(assetTypeMeta(null)).toEqual({ label: 'Unknown', color: 'neutral' });
    expect(assetTypeMeta(undefined)).toEqual({
      label: 'Unknown',
      color: 'neutral',
    });
  });

  it('renders empty type name as-is (nullish coalescing keeps "")', () => {
    expect(assetTypeMeta('')).toEqual({ label: '', color: 'neutral' });
  });
});

describe('SAC_TAG', () => {
  it('is the standalone SAC property tag (orthogonal to the type badge)', () => {
    expect(SAC_TAG).toEqual({ label: 'SAC', color: 'brown' });
  });
});

describe('ASSET_TYPE_FILTERS', () => {
  it('exposes "All types" plus the pickable asset types (no SAC — a property filter)', () => {
    expect(ASSET_TYPE_FILTERS.map((f) => f.value)).toEqual([
      '',
      'classic_credit',
      'soroban',
    ]);
    expect(ASSET_TYPE_FILTERS[0].label).toBe('All types');
  });
});

describe('assetDisplayCode', () => {
  it('names native by its type — the ledger gives it no code', () => {
    expect(
      assetDisplayCode({ asset_type_name: 'native', asset_code: null })
    ).toBe('XLM');
  });

  it('prefers a classic code over a symbol', () => {
    expect(assetDisplayCode({ asset_code: 'USDC', symbol: 'usdc' })).toBe(
      'USDC'
    );
  });

  it('falls back to the SEP-41 symbol when there is no classic code', () => {
    expect(
      assetDisplayCode({ asset_type_name: 'soroban', symbol: 'KALE' })
    ).toBe('KALE');
  });

  // A soroban token can publish no symbol at all, and its contract IS its
  // identity — showing the address names it, where a dash claims the row is
  // empty.
  it('falls back to the truncated contract address', () => {
    expect(
      assetDisplayCode({
        asset_type_name: 'soroban',
        contract_id: 'CAQCFVLOBK5GIULPNZRGSXFPMIDUTBDDKCEHQNCZGYNK5JEN6IY5RZQB',
      })
    ).toBe('CAQC…RZQB');
  });

  it('treats an empty code as absent — the shape the ledger writes native as', () => {
    expect(assetDisplayCode({ asset_code: '', symbol: 'FALLBACK' })).toBe(
      'FALLBACK'
    );
  });

  it('returns null only when nothing identifies the asset', () => {
    expect(assetDisplayCode({})).toBeNull();
  });
});
