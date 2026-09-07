import { isAccountId } from '@rumblefish/soroban-block-explorer-ui';

/**
 * SEP-2 federated addresses, both directions, resolved in the browser.
 *
 * Forward (`type=name`): `karol*lobstr.co` → `G…`, for the search box.
 * Reverse (`type=id`): `G…` → the name a domain claims for it, for the
 * account and transaction pages.
 *
 * Both hops are public GETs that serve `Access-Control-Allow-Origin: *`
 * (SEP-1 requires it for `stellar.toml`, and federation servers follow), so
 * the API never issues the request — which is the whole reason this is cheap.
 * The domain is attacker-controlled in the ordinary case, so a server-side
 * version would carry an SSRF surface; here the request leaves the user's own
 * browser and our infrastructure is not in the path at all. See task 0443.
 *
 * The trade that buys: the domain learns the viewer's IP and which account
 * they are looking at. `no-referrer` keeps our origin and path out of it, but
 * the IP is inherent to the browser making the call — accepted deliberately,
 * not overlooked.
 */

/**
 * `name*domain.tld`. Exactly one `*`, no whitespace, and a domain with at
 * least one dot and a plausible TLD — a query that merely contains a star
 * must not cost a network round-trip.
 *
 * Against SEP-2 verbatim: the username is "printable UTF-8 with whitespace
 * and the following characters excluded: `*`, `>`" — hence `[^*\s>]`, which
 * keeps the `@` of an email and the `+` of an E.164 phone number, both of
 * which the spec lists as valid names. Unicode letters are matched so an
 * internationalized domain (`münchen.de`) classifies; `fetch` converts it to
 * punycode itself.
 *
 * Deliberately STRICTER than the spec in two places, both load-bearing:
 *
 * 1. **At least one dot.** SEP-2 allows "any valid RFC 1035 domain name",
 *    which includes single-label names. In a browser a single label resolves
 *    against the VIEWER'S OWN network — `localhost`, or an intranet host like
 *    `wiki` or `nas` — so accepting them would turn a search box into a probe
 *    of the user's LAN. Nothing real is lost: all 7488 accounts whose
 *    `home_domain` has no dot hold free text like `NUMBERLAND` or
 *    `Quantum Stellar Network` (measured 2026-09-02), and a single-label host
 *    cannot serve a public HTTPS `stellar.toml` anyway.
 *
 * 2. **A two-letter minimum TLD.** RFC 1035 permits a one-character label, but
 *    no such TLD has ever been delegated and ICANN policy forbids one. The
 *    minimum is what stops a dial at `lobstr.c` on the way to `.co` or `.com`:
 *    every character this regex accepts early is a request sent to a domain
 *    the user may not have finished typing.
 */
const DOMAIN_SRC =
  '(?:[\\p{L}\\p{N}](?:[\\p{L}\\p{N}-]*[\\p{L}\\p{N}])?\\.)+\\p{L}{2,}';

const FEDERATED = new RegExp(`^[^*\\s>]+\\*(${DOMAIN_SRC})$`, 'iu');

/**
 * The domain half on its own, for `home_domain` — which is free text written
 * by the account holder, not a validated field. Measured in production:
 * 7484 accounts carry a `home_domain` with no dot in it at all, the commonest
 * being `Bankless`, `Indonesia`, `localhost:4000`, `1` and a bare space.
 * Without this gate every one of those account pages would dial a host that
 * cannot exist.
 */
const DOMAIN = new RegExp(`^${DOMAIN_SRC}$`, 'iu');

/** Budget for a whole lookup, both hops together. */
const TIMEOUT_MS = 8_000;

const FEDERATION_SERVER =
  /^[ \t]*FEDERATION_SERVER[ \t]*=[ \t]*["']([^"'\n]+)["']/im;

export type FederationResolve =
  | { kind: 'resolved'; accountId: string }
  | { kind: 'failed'; reason: string };

/** The domain half of a federated address, or `null` when `q` is not one. */
export function federatedDomain(q: string): string | null {
  const m = FEDERATED.exec(q.trim());
  return m ? m[1].toLowerCase() : null;
}

/**
 * The caller's cancellation and our own timeout, whichever fires first.
 * React Query hands a signal down and aborts it when a key is superseded;
 * without honouring it, every keystroke's lookup runs to completion against
 * a third-party host after its result has already been thrown away.
 */
function budget(signal?: AbortSignal): AbortSignal {
  const timeout = AbortSignal.timeout(TIMEOUT_MS);
  return signal == null ? timeout : AbortSignal.any([signal, timeout]);
}

/**
 * A `stellar.toml` is a config file and a federation answer is one small JSON
 * object. Anything past this is a host trying to fill the tab's memory — and
 * the request timeout does NOT bound bytes, since a fast server can stream a
 * great deal in eight seconds.
 */
async function getText(url: string, signal: AbortSignal): Promise<string> {
  const res = await fetch(url, {
    signal,
    redirect: 'follow',
    // SEP-2: "Federation responses should not be cached." Telling React Query
    // is not enough — without this the browser's own HTTP cache would still
    // honour whatever `Cache-Control` the third party sent.
    cache: 'no-store',
    // Third-party host: it has no business knowing which page asked, and
    // cookies must never ride along.
    referrerPolicy: 'no-referrer',
    credentials: 'omit',
  });
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  // ponytail: no size cap. A hostile host could stream forever, but it is a
  // host this viewer aimed at themselves, the damage stops at their own tab,
  // and `budget()` aborts the whole lookup after TIMEOUT_MS either way. The
  // cap that stood here was 30 lines of reader loop that no test could reach,
  // because a stubbed Response has no `body` to stream. If a real case for it
  // appears, put it back WITH a test that streams.
  return res.text();
}

// No cache below this line on purpose: React Query already caches the whole
// resolve per address (`federationPolicy.staleTime`), and a module-level
// per-domain cache proved worse than none — an aborted first hop would have
// been stored as "no stellar.toml" and served for its TTL.
type ServerLookup = { server: string } | { kind: 'failed'; reason: string };

async function lookupServer(
  domain: string,
  signal: AbortSignal
): Promise<ServerLookup> {
  let toml: string;
  try {
    toml = await getText(`https://${domain}/.well-known/stellar.toml`, signal);
  } catch {
    return {
      kind: 'failed',
      reason: `${domain} did not serve a stellar.toml, so its federated addresses cannot be resolved.`,
    };
  }

  const declared = FEDERATION_SERVER.exec(toml)?.[1];
  if (declared == null) {
    return {
      kind: 'failed',
      reason: `${domain} publishes a stellar.toml but no federation server, so it cannot resolve names.`,
    };
  }

  let server: URL;
  try {
    server = new URL(declared);
  } catch {
    return {
      kind: 'failed',
      reason: `${domain} declares a federation server that is not a valid URL.`,
    };
  }
  // HTTPS only, on both hops. A plaintext federation answer decides which
  // account the user is sent to, so anyone on the path could redirect them.
  if (server.protocol !== 'https:') {
    return {
      kind: 'failed',
      reason: `${domain} declares a non-HTTPS federation server, which is not called.`,
    };
  }
  return { server: server.href };
}

/** The federation server's URL for one query, built fresh per lookup. */
function query(server: string, q: string, type: 'name' | 'id'): string {
  const url = new URL(server);
  url.searchParams.set('q', q);
  url.searchParams.set('type', type);
  return url.toString();
}

/**
 * Resolve a federated address to its account id.
 *
 * `domain` comes from the caller's own `federatedDomain(address)` — the two
 * always travel together and re-deriving it here would only add a branch no
 * caller can reach.
 *
 * Never throws: every failure comes back as `failed` with a reason the UI
 * shows verbatim. An unresolvable address must not degrade into an empty
 * results page — "no results" claims the address does not exist, which is a
 * different and usually false statement.
 */
export async function resolveFederated(
  address: string,
  domain: string,
  signal?: AbortSignal
): Promise<FederationResolve> {
  const q = address.trim();
  const budgeted = budget(signal);

  const found = await lookupServer(domain, budgeted);
  if ('kind' in found) return found;

  try {
    const body = await getText(query(found.server, q, 'name'), budgeted);
    const accountId = (JSON.parse(body) as { account_id?: unknown }).account_id;
    // The server is run by the domain owner and can answer with anything.
    // Shape-check before this value becomes a route.
    if (typeof accountId === 'string' && isAccountId(accountId)) {
      return { kind: 'resolved', accountId };
    }
    return {
      kind: 'failed',
      reason: `The federation server for ${domain} answered with something that is not a Stellar account address.`,
    };
  } catch {
    return {
      kind: 'failed',
      reason: `The federation server for ${domain} did not resolve ${q}.`,
    };
  }
}

/**
 * Reverse (`type=id`): the federated address a domain claims for an account,
 * or `null` when there is none.
 *
 * Only asked for accounts that set a `home_domain` on-ledger, which is what
 * makes the answer trustworthy: the account names the domain, and the domain
 * must name the account back inside its OWN namespace — without the suffix
 * check any domain could claim an account into someone else's.
 *
 * Silent on failure, unlike the forward direction. Nobody typed this: it is
 * an attribute the page shows when it exists, so its absence is the ordinary
 * case and not a claim about anything (issue #363).
 */
export async function resolveFederatedName(
  accountId: string,
  homeDomain: string,
  signal?: AbortSignal
): Promise<string | null> {
  const domain = homeDomain.trim().toLowerCase();
  if (!DOMAIN.test(domain)) return null;

  const budgeted = budget(signal);
  const found = await lookupServer(domain, budgeted);
  if ('kind' in found) return null;

  try {
    const body = await getText(query(found.server, accountId, 'id'), budgeted);
    const address = (JSON.parse(body) as { stellar_address?: unknown })
      .stellar_address;
    return typeof address === 'string' &&
      address.toLowerCase().endsWith(`*${domain}`)
      ? address
      : null;
  } catch {
    return null;
  }
}
