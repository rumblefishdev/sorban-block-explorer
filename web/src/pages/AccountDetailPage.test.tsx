import type {
  AccountBalance,
  AccountDetailResponse,
} from '@rumblefish/api-types';
import { screen, within } from '@testing-library/react';
import { userEvent } from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../test-utils.js';

import AccountDetailPage from './AccountDetailPage.js';

const hookMocks = vi.hoisted(() => ({
  useAccountDetail: vi.fn(),
  useAccountTransactions: vi.fn(),
}));

vi.mock('../api/index.js', async () => ({
  useAccountDetail: hookMocks.useAccountDetail,
  useAccountTransactions: hookMocks.useAccountTransactions,
  // Pure pagination helper — keep the real implementation so the
  // transactions section renders instead of hitting its error boundary.
  usePagedRows: (
    await vi.importActual<typeof import('../api/usePagedRows.js')>(
      '../api/usePagedRows.js'
    )
  ).usePagedRows,
}));

const VALID_ACCOUNT =
  'GDQP2KPQGKIHYJGXNUIYOMHARUARCA7DJT5FO2FFOOUJ3K4MOMNGEE36';
const USDC_ISSUER = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN';

const NATIVE_BALANCE: AccountBalance = {
  asset_type_name: 'native',
  asset_code: null,
  asset_issuer: null,
  // RAW Int128 (task 0331) — 500 XLM scaled by decimals=7.
  balance: '5000000000',
  decimals: 7,
  last_updated_ledger: 100,
  type: 0,
  sac_deployed: false,
};
const USDC_BALANCE: AccountBalance = {
  asset_type_name: 'credit_alphanum4',
  asset_code: 'USDC',
  asset_issuer: USDC_ISSUER,
  // RAW Int128 (task 0331) — 1250.5 USDC scaled by decimals=7.
  balance: '12505000000',
  decimals: 7,
  last_updated_ledger: 100,
  type: 1,
  sac_deployed: false,
};

const SAMPLE: AccountDetailResponse = {
  account_id: VALID_ACCOUNT,
  balances: [NATIVE_BALANCE, USDC_BALANCE],
  first_seen_ledger: 50,
  last_seen_ledger: 200,
  sequence_number: 99_123_456,
  deleted: false,
};

const DELETED_SAMPLE: AccountDetailResponse = {
  ...SAMPLE,
  deleted: true,
};

/**
 * A Soroban token balance — the holding that does NOT imply an account.
 *
 * A SEP-41 balance is a `ContractData` entry owned by the TOKEN contract and
 * keyed by address, so `account_merge` never touches it and an address that
 * was never funded can hold one. Classic payments cannot reach a non-account
 * at all, which is why the signer tripwire reads classic holdings only.
 */
const SOROBAN_BALANCE: AccountBalance = {
  asset_type_name: 'pool_share', // mislabelled by the API — task 0496
  asset_code: null,
  asset_issuer: null,
  contract_id: 'CDHVVCZ46PXDTGZ6NZJCS4544ZHYIVROAOWEDJNXYXPYA2MPWXQFMTR5',
  name: 'Pool Share Token',
  symbol: 'POOL',
  balance: '120000000',
  decimals: 7,
  last_updated_ledger: 100,
  type: 3,
  sac_deployed: false,
};

function mockDetail(value: unknown): void {
  hookMocks.useAccountDetail.mockReturnValue(value);
}

beforeEach(() => {
  hookMocks.useAccountDetail.mockReset();
  // Default the transactions hook so the test never blows up when the
  // section renders — page-level assertions don't care about tx rows.
  hookMocks.useAccountTransactions.mockReturnValue({
    data: { data: [], page: { limit: 20 } },
    isLoading: false,
    isError: false,
    error: null,
    refetch: vi.fn(),
  });
});

afterEach(() => {
  vi.clearAllMocks();
});

/** N zero-balance classic assets, codes ascending so order is checkable. */
function manyZeroAssets(n: number): AccountBalance[] {
  return Array.from({ length: n }, (_, i) => ({
    ...USDC_BALANCE,
    asset_code: `A${String(i).padStart(3, '0')}`,
    balance: '0',
  }));
}

/** The Assets card alone — the transactions section below has its own pager. */
function assetsCard() {
  const card = screen.getByText('Assets').closest('.MuiCard-root');
  if (!card) throw new Error('Assets card not found');
  return within(card as HTMLElement);
}

describe('AccountDetailPage', () => {
  it('renders NotFoundState for a malformed account id (and skips the fetch)', () => {
    renderWithProviders(<AccountDetailPage />, {
      initialEntries: ['/accounts/not-a-strkey'],
      routePath: '/accounts/:accountId',
    });

    expect(screen.getByText('Account not found')).toBeInTheDocument();
    // The hook is still called with the empty fallback (guard string),
    // never with the malformed input.
    expect(hookMocks.useAccountDetail).toHaveBeenCalledWith('');
  });

  it('renders the summary + balances for a valid account', () => {
    mockDetail({
      data: SAMPLE,
      isLoading: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    });

    renderWithProviders(<AccountDetailPage />, {
      initialEntries: [`/accounts/${VALID_ACCOUNT}`],
      routePath: '/accounts/:accountId',
    });

    expect(
      screen.getByRole('heading', { level: 1, name: 'Account' })
    ).toBeInTheDocument();
    // Full account id rendered in at least one place (header sub-line +
    // copy-link inside the Summary card both repeat it).
    expect(screen.getAllByText(VALID_ACCOUNT).length).toBeGreaterThan(0);
    // Native balance always shows the "Stellar Lumens" name.
    expect(screen.getByText('Stellar Lumens')).toBeInTheDocument();
    // RAW balance is scaled by decimals for display (task 0331): native
    // `5000000000` / 7 → `500.00`; classic `12505000000` / 7 → `1,250.50`.
    expect(screen.getByText('500.00')).toBeInTheDocument();
    expect(screen.getByText('1,250.50')).toBeInTheDocument();
    // Classic credit balance shows the code as the row name.
    expect(screen.getAllByText('USDC').length).toBeGreaterThan(0);
  });

  it('counts the assets and, separately, how many carry value', () => {
    // A card titled "Balances" listing thousands of zeros argues with its own
    // contents — the zeros are real holdings (issue #377), so the card is
    // "Assets" and the two numbers are stated apart.
    mockDetail({
      data: {
        ...SAMPLE,
        balances: [NATIVE_BALANCE, { ...USDC_BALANCE, balance: '0' }],
      },
      isLoading: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    });

    renderWithProviders(<AccountDetailPage />, {
      initialEntries: [`/accounts/${VALID_ACCOUNT}`],
      routePath: '/accounts/:accountId',
    });

    expect(screen.getByText('Assets')).toBeInTheDocument();
    expect(screen.getByText('2 assets · 1 with a balance')).toBeInTheDocument();
  });

  it('drops the second clause when every asset carries value', () => {
    // Restating the same number twice reads as bureaucracy, not information.
    mockDetail({
      data: SAMPLE,
      isLoading: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    });

    renderWithProviders(<AccountDetailPage />, {
      initialEntries: [`/accounts/${VALID_ACCOUNT}`],
      routePath: '/accounts/:accountId',
    });

    expect(screen.getByText('2 assets')).toBeInTheDocument();
  });

  it('renders the assets in the order the API returned them', () => {
    // The server pins native, then funded before empty, then size, then
    // recency. Re-sorting here would put a page boundary somewhere other than
    // where the server put it.
    const zeroA = { ...USDC_BALANCE, asset_code: 'ZZZA', balance: '0' };
    const zeroB = { ...USDC_BALANCE, asset_code: 'AAAB', balance: '0' };
    mockDetail({
      data: { ...SAMPLE, balances: [NATIVE_BALANCE, zeroA, zeroB] },
      isLoading: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    });

    renderWithProviders(<AccountDetailPage />, {
      initialEntries: [`/accounts/${VALID_ACCOUNT}`],
      routePath: '/accounts/:accountId',
    });

    // Compared by position in the rendered text: each code also appears as the
    // ticker under its amount, so counting elements would double them.
    const text = document.body.textContent ?? '';
    expect(text.indexOf('Stellar Lumens')).toBeLessThan(text.indexOf('ZZZA'));
    expect(text.indexOf('ZZZA')).toBeLessThan(text.indexOf('AAAB'));
  });

  it('shows no pager at all when everything fits on one page', () => {
    // 99% of accounts hold 18 assets or fewer. They should see no hint that a
    // paging mechanism exists.
    mockDetail({
      data: SAMPLE,
      isLoading: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    });

    renderWithProviders(<AccountDetailPage />, {
      initialEntries: [`/accounts/${VALID_ACCOUNT}`],
      routePath: '/accounts/:accountId',
    });

    expect(
      assetsCard().queryByRole('button', { name: 'Next' })
    ).not.toBeInTheDocument();
  });

  // 45 rows through three pages of MUI reconciliation, with two userEvent
  // clicks: genuinely more work than the 5s default allows when the suite runs
  // lint, typecheck and tests in parallel. It timed out there while passing on
  // its own, so the limit is raised for this case rather than the test being
  // made to assert less.
  it('pages a long list and states the exact position, not "latest results"', async () => {
    // The whole set is on the page, so the caption can count — which is the
    // difference between paginating and silently capping.
    mockDetail({
      data: { ...SAMPLE, balances: manyZeroAssets(45) },
      isLoading: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    });

    const user = userEvent.setup();
    renderWithProviders(<AccountDetailPage />, {
      initialEntries: [`/accounts/${VALID_ACCOUNT}`],
      routePath: '/accounts/:accountId',
    });

    // Each code renders twice per row — as the name and as the ticker under
    // the amount — so presence is counted, not asserted as a single element.
    expect(screen.getByText('1–20 of 45')).toBeInTheDocument();
    expect(screen.getAllByText('A000').length).toBeGreaterThan(0);
    expect(screen.queryAllByText('A020')).toHaveLength(0);

    await user.click(assetsCard().getByRole('button', { name: 'Next' }));
    expect(screen.getByText('21–40 of 45')).toBeInTheDocument();
    expect(screen.getAllByText('A020').length).toBeGreaterThan(0);
    expect(screen.queryAllByText('A000')).toHaveLength(0);

    // The last page is short, and the caption says so rather than rounding up.
    await user.click(assetsCard().getByRole('button', { name: 'Next' }));
    expect(screen.getByText('41–45 of 45')).toBeInTheDocument();
    expect(assetsCard().getByRole('button', { name: 'Next' })).toBeDisabled();
  }, 15_000);

  it('opens on the page the URL names, so a position can be sent to someone', () => {
    // Every other paginated section here keeps its position in the URL. This
    // one is an offset rather than a cursor, but the property is the same:
    // survives a reload, and the link means what it showed.
    mockDetail({
      data: { ...SAMPLE, balances: manyZeroAssets(45) },
      isLoading: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    });

    renderWithProviders(<AccountDetailPage />, {
      initialEntries: [`/accounts/${VALID_ACCOUNT}?assets=3`],
      routePath: '/accounts/:accountId',
    });

    expect(screen.getByText('41–45 of 45')).toBeInTheDocument();
  });

  it('clamps a page number past the end instead of rendering nothing', () => {
    // A pasted number, or the param surviving a move to a smaller account.
    // An empty card would read as "this account holds nothing".
    mockDetail({
      data: { ...SAMPLE, balances: manyZeroAssets(45) },
      isLoading: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    });

    renderWithProviders(<AccountDetailPage />, {
      initialEntries: [`/accounts/${VALID_ACCOUNT}?assets=999`],
      routePath: '/accounts/:accountId',
    });

    expect(screen.getByText('41–45 of 45')).toBeInTheDocument();
    expect(screen.getAllByText('A044').length).toBeGreaterThan(0);
  });

  it('tags a classic balance whose SAC is deployed, and leaves the type chip alone', () => {
    // Two orthogonal axes (ADR 0051): the type chip stays "Classic credit",
    // the SAC facet is a SECOND tag. Before this field existed the page
    // inferred the facet from the issuer address starting with `C`, which
    // `asset_issuer` never is — so the tag could not render even once.
    mockDetail({
      data: {
        ...SAMPLE,
        balances: [{ ...USDC_BALANCE, sac_deployed: true }],
      },
      isLoading: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    });

    renderWithProviders(<AccountDetailPage />, {
      initialEntries: [`/accounts/${VALID_ACCOUNT}`],
      routePath: '/accounts/:accountId',
    });

    expect(screen.getByText('SAC')).toBeInTheDocument();
    expect(screen.getByText('Classic credit')).toBeInTheDocument();
  });

  it('leaves the SAC tag off native XLM, where it would be a constant', () => {
    // Reversed deliberately. `/assets` tags XLM and that is right there — the
    // question that page answers is which assets have a SAC. Here the question
    // is what this account holds, and every account holds XLM, which always
    // has one: the tag would appear on every account page forever and say
    // nothing about any of them. It earns its place on a classic row because
    // only 3,838 of 306,051 asset identities carry a deployed SAC.
    mockDetail({
      data: {
        ...SAMPLE,
        balances: [{ ...NATIVE_BALANCE, sac_deployed: true }],
      },
      isLoading: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    });

    renderWithProviders(<AccountDetailPage />, {
      initialEntries: [`/accounts/${VALID_ACCOUNT}`],
      routePath: '/accounts/:accountId',
    });

    expect(screen.getByText('Stellar Lumens')).toBeInTheDocument();
    expect(screen.queryByText('SAC')).not.toBeInTheDocument();
  });

  it('shows no SAC tag for a classic balance without a deployed one', () => {
    mockDetail({
      data: { ...SAMPLE, balances: [USDC_BALANCE] },
      isLoading: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    });

    renderWithProviders(<AccountDetailPage />, {
      initialEntries: [`/accounts/${VALID_ACCOUNT}`],
      routePath: '/accounts/:accountId',
    });

    // A reserved-but-undeployed SAC is an address, not a contract — no tag.
    expect(screen.queryByText('SAC')).not.toBeInTheDocument();
    expect(screen.getByText('Classic credit')).toBeInTheDocument();
  });

  it('links a classic-credit balance to its /assets/:id detail page', () => {
    mockDetail({
      data: SAMPLE,
      isLoading: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    });

    renderWithProviders(<AccountDetailPage />, {
      initialEntries: [`/accounts/${VALID_ACCOUNT}`],
      routePath: '/accounts/:accountId',
    });

    const usdcLink = screen.getByRole('link', { name: 'USDC' });
    expect(usdcLink).toHaveAttribute(
      'href',
      `/assets/${encodeURIComponent(`USDC-${USDC_ISSUER}`)}`
    );
  });

  it('shows a Deleted badge for a merged account', () => {
    mockDetail({
      data: DELETED_SAMPLE,
      isLoading: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    });

    renderWithProviders(<AccountDetailPage />, {
      initialEntries: [`/accounts/${VALID_ACCOUNT}`],
      routePath: '/accounts/:accountId',
    });

    expect(screen.getByText('Deleted')).toBeInTheDocument();
  });

  it('omits the Deleted badge for a live account', () => {
    mockDetail({
      data: SAMPLE,
      isLoading: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    });

    renderWithProviders(<AccountDetailPage />, {
      initialEntries: [`/accounts/${VALID_ACCOUNT}`],
      routePath: '/accounts/:accountId',
    });

    expect(screen.queryByText('Deleted')).not.toBeInTheDocument();
  });

  it('a closed account holding only a Soroban token reads as Closed', () => {
    // 9,388 accounts on pubnet. The account is gone (probed ABSENT on chain)
    // but its token balance is not, and the balance matched the chain exactly
    // (60/60). Reading ALL holdings made the page announce an indexing gap
    // about correct data; reading classic holdings lets `deleted` answer.
    mockDetail({
      data: { ...DELETED_SAMPLE, balances: [SOROBAN_BALANCE] },
      isLoading: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    });

    renderWithProviders(<AccountDetailPage />, {
      initialEntries: [`/accounts/${VALID_ACCOUNT}`],
      routePath: '/accounts/:accountId',
    });

    expect(screen.getByText('Closed')).toBeInTheDocument();
    expect(screen.queryByText('Not indexed')).not.toBeInTheDocument();
  });

  it('an address that was never an account can still hold a Soroban token', () => {
    // 1,325 addresses: sequence number 0, no `AccountEntry` on chain, a live
    // token balance anyway. This is the case a mere branch reorder would NOT
    // have fixed — `deleted` is false, so only the classic-only flag helps.
    mockDetail({
      data: { ...SAMPLE, balances: [SOROBAN_BALANCE] },
      isLoading: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    });

    renderWithProviders(<AccountDetailPage />, {
      initialEntries: [`/accounts/${VALID_ACCOUNT}`],
      routePath: '/accounts/:accountId',
    });

    expect(screen.getByText('No account')).toBeInTheDocument();
    expect(screen.queryByText('Not indexed')).not.toBeInTheDocument();
    // …and the page says WHY the two facts on screen are not a contradiction.
    expect(screen.getByText(/need no account/)).toBeInTheDocument();
  });

  it('renders NotFoundState when the detail query 404s', () => {
    mockDetail({
      data: undefined,
      isLoading: false,
      isError: true,
      // `classifyError` reads `.status` to detect missing-resource.
      error: Object.assign(new Error('not found'), { status: 404 }),
      refetch: vi.fn(),
    });

    renderWithProviders(<AccountDetailPage />, {
      initialEntries: [`/accounts/${VALID_ACCOUNT}`],
      routePath: '/accounts/:accountId',
    });

    expect(screen.getByText('Account not found')).toBeInTheDocument();
  });

  it('renders GenericErrorState for a non-404 fetch error and isolates it from the transactions section', () => {
    mockDetail({
      data: undefined,
      isLoading: false,
      isError: true,
      error: Object.assign(new Error('boom'), { status: 500 }),
      refetch: vi.fn(),
    });

    renderWithProviders(<AccountDetailPage />, {
      initialEntries: [`/accounts/${VALID_ACCOUNT}`],
      routePath: '/accounts/:accountId',
    });

    // Generic error state surfaces the retry CTA.
    expect(
      screen.getByRole('button', { name: /try again/i })
    ).toBeInTheDocument();
    // Header still rendered — the parent error doesn't tear down the page.
    expect(
      within(screen.getByRole('heading', { level: 1 })).getByText('Account')
    ).toBeInTheDocument();
    // Transactions section is intentionally hidden on detail error.
    expect(screen.queryByText(/recent transactions/i)).not.toBeInTheDocument();
  });
});
