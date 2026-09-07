import { screen, waitFor } from '@testing-library/react';
import { userEvent } from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  emptySearchState,
  fetchReply,
  renderWithProviders,
  stubFetch,
} from '../test-utils.js';
import { GlobalSearchBar } from './GlobalSearchBar.js';
import type { SearchResultsState } from './useSearchResults.js';

const seen: string[] = [];

vi.mock('./useSearchResults.js', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./useSearchResults.js')>();
  return {
    ...actual,
    useSearchResults: vi.fn((params: { q: string }): SearchResultsState => {
      seen.push(params.q);
      return emptySearchState(params.q);
    }),
  };
});

function renderBar(q: string) {
  seen.length = 0;
  return renderWithProviders(
    <GlobalSearchBar
      q={q}
      onDismiss={() => undefined}
      registerEnterHandler={() => undefined}
    />
  );
}

const ACCOUNT = 'GC526FUILJ6NLFXKCOOGTMDXNRW7MYSEK2UNRJV5FYWOGYDE4LOKXFEM';

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('GlobalSearchBar — federated addresses (task 0443 scope A)', () => {
  // `/v1/search` knows nothing about SEP-2, so it answers zero hits and the
  // dropdown would otherwise say "No results for karol*lobstr.co" — the one
  // claim that is false about an address that resolves.
  // The whole point of the arming rule: a half-typed address must not dial
  // anyone. `lobstr.co` is a real domain on the way to `lobstr.com`.
  it('sends nothing until the row is picked', async () => {
    const fetchMock = stubFetch({});

    renderBar('karol*lobstr.co');

    expect(screen.getByText(/look it up with lobstr\.co/)).toBeInTheDocument();
    expect(screen.queryByText(/No results/)).not.toBeInTheDocument();
    // Nothing has left the browser, and nothing will until the row is picked.
    await waitFor(() => expect(fetchMock).not.toHaveBeenCalled());
  });

  it('resolves once the row is picked, and says which domain it is asking', async () => {
    const user = userEvent.setup();
    stubFetch({
      'https://lobstr.co/.well-known/stellar.toml': fetchReply(
        'FEDERATION_SERVER="https://lobstr.co/federation/"\n'
      ),
      'https://lobstr.co/federation/': fetchReply(
        JSON.stringify({ account_id: ACCOUNT })
      ),
    });

    renderBar('karol*lobstr.co');
    await user.click(screen.getByRole('option'));

    expect(screen.getByText(/Asking lobstr\.co/)).toBeInTheDocument();
    // Picked rows do not stay pickable while their answer is in flight.
    expect(screen.getByRole('option')).toBeDisabled();
  });

  // A dead domain must say which hop failed, in the dropdown, rather than
  // leaving the row spinning or falling back to an empty list.
  it('names the failed hop when the domain serves no stellar.toml', async () => {
    const user = userEvent.setup();
    stubFetch({});

    renderBar('karol*lobstr.co');
    await user.click(screen.getByRole('option'));

    expect(
      await screen.findByText(/did not serve a stellar\.toml/)
    ).toBeInTheDocument();
  });

  // Suppressing the /v1/search request now belongs to `useSearchResults`
  // itself (it is mocked here). What this component still owns is offering
  // the row as a click target, not only as an Enter shortcut.
  it('offers the federated row as a click target', () => {
    stubFetch({});
    renderBar('karol*lobstr.co');

    const row = screen.getByRole('option');
    expect(row.tagName).toBe('BUTTON');
  });

  it('leaves an ordinary query on the normal results view', () => {
    const fetchMock = stubFetch({});
    renderBar('kale');

    expect(screen.queryByText(/look it up with/)).not.toBeInTheDocument();
    // An ordinary query must not reach a third-party host at all.
    expect(fetchMock).not.toHaveBeenCalled();
    expect(seen).toContain('kale');
  });
});
