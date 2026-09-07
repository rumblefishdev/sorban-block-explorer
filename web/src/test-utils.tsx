import { ExplorerThemeProvider } from '@rumblefish/soroban-block-explorer-ui';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import {
  render,
  type RenderOptions,
  type RenderResult,
} from '@testing-library/react';
import type { ReactElement, ReactNode } from 'react';
import { vi } from 'vitest';
import { MemoryRouter, Route, Routes } from 'react-router-dom';

import type { SearchResultsState } from './search/useSearchResults.js';

interface RenderWithProvidersOptions extends Omit<RenderOptions, 'wrapper'> {
  initialEntries?: string[];
  routePath?: string;
  queryClient?: QueryClient;
}

/**
 * Renders a UI tree inside the providers every page expects:
 * `QueryClientProvider`, `ExplorerThemeProvider`, and `MemoryRouter`.
 * Pass `routePath` (e.g. `/accounts/:accountId`) together with
 * `initialEntries` to drive `useParams` in page-component tests.
 */
export function renderWithProviders(
  ui: ReactElement,
  options: RenderWithProvidersOptions = {}
): RenderResult & { queryClient: QueryClient } {
  const {
    initialEntries = ['/'],
    routePath,
    queryClient = makeTestQueryClient(),
    ...rest
  } = options;

  function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={queryClient}>
        <ExplorerThemeProvider>
          <MemoryRouter initialEntries={initialEntries}>
            {routePath != null ? (
              <Routes>
                <Route path={routePath} element={children} />
              </Routes>
            ) : (
              children
            )}
          </MemoryRouter>
        </ExplorerThemeProvider>
      </QueryClientProvider>
    );
  }

  const result = render(ui, { wrapper: Wrapper, ...rest });
  return { ...result, queryClient };
}

/**
 * QueryClient tuned for tests — no retries, no caching, no stale window
 * so each test gets deterministic fetch behaviour.
 */
export function makeTestQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: 0, staleTime: 0 },
      mutations: { retry: false },
    },
  });
}

/**
 * Shared stubs for the SEP-2 federation tests (task 0443). They lived in
 * three files with small drifts — one copy had silently lost the `ok` flag,
 * so two helpers that looked interchangeable were not.
 */

/** One canned HTTP response for `stubFetch`. */
export function fetchReply(body: string, ok = true) {
  return { ok, status: ok ? 200 : 404, text: () => Promise.resolve(body) };
}

/**
 * Serve each URL prefix from a map; anything unmapped rejects the way a dead
 * host does. Returns the mock so a test can assert on the calls.
 */
export function stubFetch(
  routes: Record<string, ReturnType<typeof fetchReply>>
) {
  const fn = vi.fn((url: string) => {
    const hit = Object.entries(routes).find(([prefix]) =>
      url.startsWith(prefix)
    );
    return hit
      ? Promise.resolve(hit[1])
      : Promise.reject(new Error('ENOTFOUND'));
  });
  vi.stubGlobal('fetch', fn);
  return fn;
}

/** A `SearchResultsState` with nothing found, for tests that mock the hook. */
export function emptySearchState(
  q: string,
  overrides: Partial<SearchResultsState> = {}
): SearchResultsState {
  return {
    effectiveQuery: q,
    data: undefined,
    isFetching: false,
    isError: false,
    error: null,
    refetch: () => undefined,
    counts: {
      transaction: 0,
      account: 0,
      contract: 0,
      asset: 0,
      nft: 0,
      pool: 0,
    },
    totalCount: 0,
    activeTab: 'transaction',
    setActiveTab: () => undefined,
    hitsForActiveTab: [],
    ...overrides,
  };
}
