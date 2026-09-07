import { useQuery } from '@tanstack/react-query';
import { useCallback, useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import { routes } from '../router/routes.js';

import { federationPolicy } from '../api/polling.js';

import {
  federatedDomain,
  resolveFederated,
  resolveFederatedName,
} from './federation.js';
import type { FederationResolve } from './federation.js';

/**
 * The SEP-2 lookups as named hooks, so fetching stays out of the
 * components that render it — the seam every other query in this app already
 * uses (`web/src/api/hooks/`). It lives here rather than there because it
 * do not talk to our API at all, and their policy lives in `api/polling.ts` as
 * `federationPolicy`.
 *
 * Passes React Query's `AbortSignal` down. Without it a superseded lookup
 * keeps running against a third-party host after its result is discarded.
 */

/**
 * Forward: a federated address the user typed → the account it names.
 *
 * Fires only when `armed` — never while typing. Debouncing was the wrong tool
 * here: any delay short enough to feel live still dials on a pause mid-word,
 * and `lobstr.co` is a real domain on the way to `lobstr.com`. Measured in our
 * own data: 22 domains are a literal prefix of another, `google.co`/`.com` and
 * `doge-token.co`/`.com` among them, with different owners. So the request
 * waits for an explicit act — a click on the row, or Enter — and until then
 * this leaves for no host at all.
 *
 * The caller arms per query string, so editing the text disarms it again.
 *
 * A caller that is armed while its text can still change — the results page,
 * where landing is the commit but the input stays editable — must settle the
 * value first; see `FEDERATION_SETTLE_MS`.
 */
/**
 * How long an armed-but-editable caller waits before asking. Longer than the
 * app-wide 300 ms: this request leaves for a host the typed text names, so a
 * pause mid-word must not be enough to dial it.
 */
export const FEDERATION_SETTLE_MS = 500;

export function useFederatedAddress(
  address: string,
  armed: boolean
): {
  /** The domain being asked, or `null` when the input is not federated. */
  domain: string | null;
  data: FederationResolve | undefined;
} {
  const settled = address.trim();
  const domain = federatedDomain(settled);
  const query = useQuery({
    queryKey: ['federatedAddress', settled],
    queryFn: ({ signal }) => resolveFederated(settled, domain ?? '', signal),
    enabled: armed && domain != null,
    ...federationPolicy,
  });
  return { domain, data: query.data };
}

/**
 * Reverse: an account and its on-ledger home domain → the address that domain
 * claims for it, `null` when there is none, `undefined` while it is being
 * asked. No debounce here — the input is an account id from a loaded page,
 * not something being typed.
 */
export function useFederatedName(
  accountId: string,
  homeDomain: string
): string | null | undefined {
  return useQuery({
    queryKey: ['federatedName', accountId, homeDomain],
    queryFn: ({ signal }) =>
      resolveFederatedName(accountId, homeDomain, signal),
    enabled: homeDomain.length > 0,
    ...federationPolicy,
  }).data;
}

/** What both search surfaces need to draw and drive a federated lookup. */
export interface FederatedLookup {
  /** The domain being offered or asked, `null` when `q` is not an address. */
  domain: string | null;
  /** True once the user has asked. Until then nothing has left the browser. */
  armed: boolean;
  /** Ask the domain about the current query. */
  ask: () => void;
  /** Why the lookup failed, verbatim for the user, or `null`. */
  failure: string | null;
}

/**
 * The whole forward flow behind one call: classify, hold the "may we ask yet"
 * decision, and go to the account when the answer arrives.
 *
 * Both surfaces run this. They differ in exactly one thing — whether the
 * query they were handed has already been asked for — so that is the one
 * parameter. The header dropdown starts unarmed, because its text is being
 * typed. The results page starts armed, because arriving at `/search?q=…` is
 * itself the act of asking: the user pressed Enter, picked the row, or pasted
 * the link. Editing disarms either of them, since arming is keyed to the
 * exact query and `bob*lobstr.com` passes through `bob*lobstr.co` — a real
 * domain with a different owner.
 *
 * Classification runs on the live text: it only decides which panel to draw,
 * costs a regex and reaches no network. Asking runs on the armed text.
 */
export function useFederatedLookup(
  q: string,
  { askOnMount, onResolved }: { askOnMount: boolean; onResolved?: () => void }
): FederatedLookup {
  const trimmed = q.trim();
  const navigate = useNavigate();
  const [askedFor, setAskedFor] = useState<string | null>(() =>
    askOnMount ? trimmed : null
  );
  const armed = askedFor != null && askedFor === trimmed;
  const { domain, data } = useFederatedAddress(q, armed);

  const resolved = armed && data?.kind === 'resolved' ? data.accountId : null;
  const failure = armed && data?.kind === 'failed' ? data.reason : null;

  useEffect(() => {
    if (resolved == null) return;
    onResolved?.();
    navigate(routes.account(resolved), { replace: askOnMount });
  }, [resolved, navigate, onResolved, askOnMount]);

  return {
    domain,
    armed,
    ask: useCallback(() => setAskedFor(trimmed), [trimmed]),
    failure,
  };
}
