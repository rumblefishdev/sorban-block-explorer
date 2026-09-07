import type { TransactionListItem } from '@rumblefish/api-types';
import { screen } from '@testing-library/react';
import { userEvent } from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../test-utils.js';

import TransactionsListPage from './TransactionsListPage.js';

const hookMocks = vi.hoisted(() => ({
  useTransactionsList: vi.fn(),
}));

vi.mock('../api/hooks/useTransactionsList.js', () => ({
  useTransactionsList: hookMocks.useTransactionsList,
}));

function makeTx(
  overrides: Partial<TransactionListItem> = {}
): TransactionListItem {
  return {
    hash: '7b2a8c1f9d4e6a3b8c1f9d4e6a3b8c1f9d4e6a3b8c1f9d4e6a3b8c1f9d4e6a3b',
    ledger_sequence: 54_837_201,
    application_order: 1,
    fee_charged: 100,
    successful: true,
    operation_count: 1,
    has_soroban: false,
    operation_types: ['PAYMENT'],
    created_at: new Date(Date.now() - 60_000).toISOString(),
    source_account: 'GBQFOPGGSP4VCDFXJ4YEPCQNLN6EFRC4M7OOLQOEEY7H4VPF6N4WEE2N',
    inner_tx_hash: null,
    ...overrides,
  };
}

function mockOk(rows: TransactionListItem[]): void {
  hookMocks.useTransactionsList.mockReturnValue({
    data: {
      data: rows,
      page: { limit: 20, next_cursor: null, prev_cursor: null },
    },
    isLoading: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  });
}

beforeEach(() => {
  hookMocks.useTransactionsList.mockReset();
});

afterEach(() => {
  vi.clearAllMocks();
});

describe('TransactionsListPage', () => {
  it('renders the page header and the first transaction row', () => {
    mockOk([makeTx()]);

    renderWithProviders(<TransactionsListPage />, {
      initialEntries: ['/transactions'],
    });

    expect(
      screen.getByRole('heading', { level: 1, name: /transactions/i })
    ).toBeInTheDocument();
    // Truncated hash (first 4 + last 4) — the single truncation standard
    // from getDefaultTruncation; single-glyph ellipsis (…) per the
    // truncateMiddle default (task 0348 finding 13).
    expect(screen.getByText('7b2a…6a3b')).toBeInTheDocument();
  });

  it('renders the empty state when zero rows come back with no filters', () => {
    mockOk([]);

    renderWithProviders(<TransactionsListPage />, {
      initialEntries: ['/transactions'],
    });

    expect(screen.getByText(/no transactions yet/i)).toBeInTheDocument();
  });

  it('normalises a lowercase URL op param and forwards it to the hook', () => {
    mockOk([]);

    renderWithProviders(<TransactionsListPage />, {
      initialEntries: ['/transactions?op=payment'],
    });

    const lastFilters =
      hookMocks.useTransactionsList.mock.calls[
        hookMocks.useTransactionsList.mock.calls.length - 1
      ]?.[1];
    expect(lastFilters?.['filter[operation_type]']).toBe('PAYMENT');
  });

  it('drops an unknown URL op param rather than sending it to the API', () => {
    mockOk([]);

    renderWithProviders(<TransactionsListPage />, {
      initialEntries: ['/transactions?op=not-a-real-op'],
    });

    const lastFilters =
      hookMocks.useTransactionsList.mock.calls[
        hookMocks.useTransactionsList.mock.calls.length - 1
      ]?.[1];
    expect(lastFilters?.['filter[operation_type]']).toBeUndefined();
  });

  it('routes a strkey search to the source-account filter', async () => {
    mockOk([]);
    const user = userEvent.setup();

    renderWithProviders(<TransactionsListPage />, {
      initialEntries: ['/transactions'],
    });

    const search = screen.getByPlaceholderText(
      /source account or contract id/i
    );
    await user.type(
      search,
      'GBQFOPGGSP4VCDFXJ4YEPCQNLN6EFRC4M7OOLQOEEY7H4VPF6N4WEE2N'
    );

    await vi.waitFor(() => {
      const lastFilters =
        hookMocks.useTransactionsList.mock.calls[
          hookMocks.useTransactionsList.mock.calls.length - 1
        ]?.[1];
      expect(lastFilters?.['filter[source_account]']).toBe(
        'GBQFOPGGSP4VCDFXJ4YEPCQNLN6EFRC4M7OOLQOEEY7H4VPF6N4WEE2N'
      );
      // Contract filter is left empty for a G-strkey.
      expect(lastFilters?.['filter[contract_id]']).toBeUndefined();
    });
  });
});
