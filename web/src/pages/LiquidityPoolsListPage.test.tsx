import type { PaginatedPoolItem, PoolItem } from '@rumblefish/api-types';
import { screen } from '@testing-library/react';
import { userEvent } from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../test-utils.js';

import LiquidityPoolsListPage from './LiquidityPoolsListPage.js';

const poolsHookMock = vi.hoisted(() => ({ usePoolsList: vi.fn() }));

vi.mock('../api/hooks/usePoolsList.js', () => ({
  usePoolsList: poolsHookMock.usePoolsList,
}));

/**
 * Native carries NO `asset_code` on the ledger — the API returns null and the
 * naming ladder is what makes it read `XLM`. A fixture that hands it `'XLM'`
 * would pass whether or not that rule survives.
 */
const NATIVE_LEG = {
  asset_type_name: 'native',
  asset_code: null,
  issuer: null,
  contract_id: null,
  sac_contract_id: null,
  icon_url: null,
};

function creditLeg(code: string): PoolItem['legs'][number] {
  return {
    ...NATIVE_LEG,
    asset_type_name: 'classic_credit',
    asset_code: code,
    issuer: 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN',
  };
}

function makePool(overrides: Partial<PoolItem> = {}): PoolItem {
  return {
    pool_id: 'L'.padEnd(56, 'A'),
    pool_kind: 'classic',
    legs: [NATIVE_LEG, creditLeg('USDC')],
    fee_bps: 30,
    fee_percent: '0.30',
    created_at_ledger: 50_000_000,
    participant_count: 12,
    latest_snapshot_ledger: 63_000_000,
    reserve_a: '1000.0000000',
    reserve_b: '250.0000000',
    total_shares: '500.0000000',
    tvl: '1250.00',
    volume: null,
    fee_revenue: null,
    latest_snapshot_at: new Date().toISOString(),
    ...overrides,
  };
}

type Page = NonNullable<PaginatedPoolItem['page']>;

function mockOk(rows: PoolItem[], page?: Partial<Page>): void {
  poolsHookMock.usePoolsList.mockReturnValue({
    data: {
      data: rows,
      page: { limit: 20, next_cursor: null, prev_cursor: null, ...page },
    },
    isLoading: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  });
}

/** The filters object handed to the hook on the most recent render. */
function lastFilters() {
  const calls = poolsHookMock.usePoolsList.mock.calls;
  return calls[calls.length - 1]?.[1];
}

beforeEach(() => {
  poolsHookMock.usePoolsList.mockReset();
});

afterEach(() => {
  vi.clearAllMocks();
});

describe('LiquidityPoolsListPage', () => {
  it('names a pool by every leg it has, not by a left and a right', () => {
    mockOk([
      makePool({
        pool_kind: 'soroban',
        pool_id: 'C'.padEnd(56, 'A'),
        legs: [creditLeg('USDC'), creditLeg('EURC'), creditLeg('DAI')],
      }),
    ]);

    renderWithProviders(<LiquidityPoolsListPage />, {
      initialEntries: ['/liquidity-pools'],
    });

    // The third leg is the whole point: the pair shape this replaced had
    // nowhere to put it, and a three-leg stable pool rendered as a two-leg one.
    expect(screen.getByText('USDC / EURC / DAI')).toBeInTheDocument();
  });

  it('shows a code-less soroban leg by its truncated contract address', () => {
    mockOk([
      makePool({
        pool_kind: 'soroban',
        pool_id: 'C'.padEnd(56, 'A'),
        legs: [
          NATIVE_LEG,
          {
            ...NATIVE_LEG,
            asset_type_name: 'soroban',
            contract_id:
              'CAQCFVLOBK5GIULPNZRGSXFPMIDUTBDDKCEHQNCZGYNK5JEN6IY5RZQB',
          },
        ],
      }),
    ]);

    renderWithProviders(<LiquidityPoolsListPage />, {
      initialEntries: ['/liquidity-pools'],
    });

    // Not a dash: the contract IS the token's identity, so it names the leg.
    expect(screen.getByText('XLM / CAQC…RZQB')).toBeInTheDocument();
  });

  it('says a pool has no indexed legs instead of naming it nothing', () => {
    mockOk([makePool({ legs: [], reserve_a: '1000.0', reserve_b: '250.0' })]);

    renderWithProviders(<LiquidityPoolsListPage />, {
      initialEntries: ['/liquidity-pools'],
    });

    // ~10.4k classic pools are in this state while the backfill runs. A blank
    // name would read as a pool that holds nothing.
    expect(screen.getByText('Composition not indexed')).toBeInTheDocument();
  });

  it('badges each row with its kind, which no other column shows', () => {
    mockOk([
      makePool({ pool_kind: 'soroban', pool_id: 'C'.padEnd(56, 'A') }),
      makePool({ pool_kind: 'classic' }),
    ]);

    renderWithProviders(<LiquidityPoolsListPage />, {
      initialEntries: ['/liquidity-pools'],
    });

    // Not the filter chips — those are buttons; these are the row badges.
    expect(screen.getAllByText('Soroban').length).toBeGreaterThan(1);
    expect(screen.getAllByText('Classic').length).toBeGreaterThan(1);
  });

  it('carries the kind chip through to filter[pool_kind]', async () => {
    mockOk([]);
    const user = userEvent.setup();

    renderWithProviders(<LiquidityPoolsListPage />, {
      initialEntries: ['/liquidity-pools'],
    });

    expect(lastFilters()?.['filter[pool_kind]']).toBeUndefined();

    await user.click(screen.getByRole('button', { name: 'Soroban' }));

    // The API rejects an unknown kind with 400, so the spelling the chip
    // sends has to be the one the backend parses — `classic` | `soroban`.
    await vi.waitFor(() => {
      expect(lastFilters()?.['filter[pool_kind]']).toBe('soroban');
    });
  });

  it('reads the kind back off the URL, so the filter survives a reload', () => {
    mockOk([]);

    renderWithProviders(<LiquidityPoolsListPage />, {
      initialEntries: ['/liquidity-pools?kind=classic'],
    });

    expect(lastFilters()?.['filter[pool_kind]']).toBe('classic');
    expect(screen.getByRole('button', { name: 'Classic' })).toHaveAttribute(
      'aria-pressed',
      'true'
    );
  });
});
