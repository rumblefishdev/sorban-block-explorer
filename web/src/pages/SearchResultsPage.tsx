import { Box, CircularProgress, Paper, Stack, Typography } from '@mui/material';
import { useEffect, useState } from 'react';
import { useNavigate, useSearchParams } from 'react-router-dom';

import { SearchInput } from '@rumblefish/soroban-block-explorer-ui';

import { directRouteFor } from '../search/directRouteFor.js';
import { SearchResultsView } from '../search/SearchResultsView.js';
import { FederationStatus } from '../search/FederationStatus.js';
import { useFederatedLookup } from '../search/useFederation.js';
import { useSearchResults } from '../search/useSearchResults.js';

export default function SearchResultsPage() {
  const [params, setParams] = useSearchParams();
  const navigate = useNavigate();
  const q = params.get('q') ?? '';

  // The input is driven by local state, not by the URL. Controlling it
  // directly off `params` put the value one render behind the keystroke, so
  // React re-assigned `input.value` afterwards and the browser dropped the
  // caret at the end — editing mid-query was impossible. The URL is still
  // written on every change and is still the shareable value; it just is no
  // longer what the field reads back. The effect exists only for history
  // navigation and remount (task 0527 #1).
  const [text, setText] = useState(q);
  useEffect(() => {
    setText(q);
  }, [q]);

  function writeQuery(next: string) {
    setText(next);
    const nextParams = new URLSearchParams(params);
    if (next.length > 0) nextParams.set('q', next);
    else nextParams.delete('q');
    setParams(nextParams, { replace: true });
  }

  // The search hook classifies the query too and suppresses its own request
  // for a federated address, so the two never disagree about what the buckets
  // should be asked.
  const state = useSearchResults({ q });

  // SEP-2 federated address (`name*domain`) typed into search — task 0443.
  // `directRouteFor` cannot carry this: it is synchronous, and the resolve is
  // two network round-trips. It hooks in here instead, the one point the
  // app-shell bar, the home hero and a pasted `/search?q=` URL all converge
  // on, so neither caller needs to change.
  //
  // The same flow the header dropdown runs (`useFederatedLookup`), so the two
  // cannot disagree about when a domain may be asked or where the answer
  // lands. This surface differs in one thing: arriving at /search?q=… is
  // itself the act of asking — the user pressed Enter, picked the row, or
  // pasted the link — so the query the page mounts with is already armed.
  // Anything typed afterwards is not.
  const federated = useFederatedLookup(q, { askOnMount: true });
  const federatedFor = federated.domain;
  const failure = federated.failure;

  // Deep-link / paste path (`/search?q=...`) bypasses the AppShell +
  // HomeHero submit handlers that normally call `directRouteFor`
  // first. Re-run the FE classifier here so a ledger sequence pasted
  // into the URL bar (or typed into this page's own `SearchInput`,
  // which writes back to the `q` param via `setParams`) lands on
  // `/ledgers/<seq>` instead of an empty broad-search results page.
  useEffect(() => {
    const target = directRouteFor(q);
    if (target) {
      navigate(target, { replace: true });
    }
  }, [q, navigate]);

  // No singleton auto-navigation. Task 0271 sent a one-hit broad search
  // straight to that hit; 0527 withdrew it — it took the page away before
  // the match could be read, and `replace: true` meant Back could not bring
  // it back. Accepted cost: a pasted full tx hash or StrKey also matched
  // exactly one row and rode this effect, so it now stops on a one-row
  // results page — one click from the detail page. Only a bare ledger
  // sequence still redirects directly (`directRouteFor` above); /v1/search
  // has had no redirect branch since 0271.

  return (
    <Stack spacing={3} sx={{ py: 2 }}>
      <Box>
        {/* Heading needs an explicit `component="h1"` — without it both
            Typography blocks default to plain divs, so the a11y tree
            concatenates them into one run ("SearchRefine your query…").
            Aligning with the other list/detail pages, which all set the
            heading element explicitly (task 0251 H10). */}
        <Typography variant="heading4Bold" component="h1" sx={{ mb: 0.5 }}>
          Search
        </Typography>
        <Typography
          variant="bodySmRegular"
          component="p"
          sx={(theme) => ({ color: theme.palette.text.tertiary })}
        >
          Refine your query to find transactions, accounts, contracts, tokens,
          NFTs, and liquidity pools.
        </Typography>
      </Box>

      <Box sx={{ maxWidth: 628 }}>
        <SearchInput
          size="lg"
          value={text}
          onChange={writeQuery}
          onClear={() => writeQuery('')}
        />
      </Box>

      {/* Rounded outlined card with a surface background (results + tab headers
          read as one block) — using the explorer's standard table/card border
          (`stroke.default`, 1px), not the nav-bar dropdown's accent border.
          A federated input takes the same card over: `/v1/search` knows
          nothing about SEP-2, so its empty answer would render as "no
          results" — a claim that the address does not exist. */}
      <Paper
        variant="outlined"
        elevation={0}
        sx={(theme) => ({
          borderColor:
            failure != null
              ? theme.palette.stroke.error
              : theme.palette.stroke.default,
          backgroundColor:
            failure != null
              ? theme.palette.surface.error
              : theme.palette.surface.grayMain,
          overflow: 'hidden',
        })}
      >
        {federatedFor == null ? (
          <SearchResultsView state={state} />
        ) : (
          <Stack spacing={1} sx={{ px: 2, py: 1.5 }}>
            <Stack direction="row" spacing={1.5} alignItems="center">
              {/* Inline marker, not the shared `SearchSpinner` — that one
                  centres itself in a full-width 80px block, which is right
                  for an empty results panel and wrong beside a line of
                  text. */}
              {federated.armed && failure == null && (
                <CircularProgress size={16} thickness={5} />
              )}
              <FederationStatus
                address={q.trim()}
                domain={federatedFor}
                armed={federated.armed}
                failure={failure}
              />
            </Stack>
            {/* Unarmed, or a failure the user may want to retry after the
                domain comes back: the same button covers both, because both
                are "ask this domain now". */}
            {!federated.armed || failure != null ? (
              <Box
                component="button"
                type="button"
                onClick={federated.ask}
                sx={(theme) => ({
                  alignSelf: 'flex-start',
                  background: 'none',
                  border: 'none',
                  p: 0,
                  cursor: 'pointer',
                  textDecoration: 'underline',
                  font: 'inherit',
                  color: theme.palette.text.primary,
                })}
              >
                {failure != null ? 'Try again' : `Ask ${federatedFor}`}
              </Box>
            ) : null}
          </Stack>
        )}
      </Paper>
    </Stack>
  );
}
