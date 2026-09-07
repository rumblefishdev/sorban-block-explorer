---
id: '0443'
title: 'FEATURE: SEP-2 federated addresses — resolve name*domain in search (A), show it on accounts (B)'
type: FEATURE
status: active
related_adr: []
related_tasks: ['0188']
tags:
  [backend, enrichment, frontend, accounts, sep2, priority-low, effort-medium]
links:
  - 'SEP-2 (Federation Protocol): https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0002.md'
  - 'https://github.com/rumblefishdev/soroban-block-explorer/issues/363'
history:
  - date: '2026-07-27'
    status: backlog
    who: karolkow
    note: >
      Spawned from external feedback on the live deployment: "if there's a
      federated address tied to this source account, it would be great to show
      it". Not implemented anywhere today — no prior task covers SEP-2.
  - date: '2026-09-01'
    status: active
    who: karolkow
    note: >
      Split into two independent scopes after a second external comment asked
      for the OTHER direction — typing `name*domain` into search. That
      direction (A) is browser-only, needs no backend and no storage, so it
      ships first; the original account-page direction (B) keeps the whole
      enrichment + SSRF cost and stays behind it. Activated for A.
  - date: '2026-09-02'
    status: active
    who: karolkow
    note: >
      Read SEP-1 and SEP-2 at the source and checked the implementation
      against them. Four corrections: internationalized domains were not
      classified at all; `>` was accepted in a username the spec excludes;
      federation answers were cached for five minutes against an explicit
      "should not be cached"; and the response-size cap two acceptance
      criteria claimed did not exist. Also gated the reverse direction on the
      domain shape — 7484 accounts carry a dotless `home_domain` (`Bankless`,
      `Indonesia`, `localhost:4000`, `1`, a bare space), every one of which
      was dialled before.
  - date: '2026-09-03'
    status: active
    who: karolkow
    note: >
      Measured the tail that the first CORS pass had left as "unknown". No
      violators in it at all: of 115 further domains (59450 accounts), 98.7%
      run no federation server, so they are not blocked, they simply have no
      answer. That drops the real cost of the CORS rule from an implied 16% of
      accounts to 1.9%, and settles the proxy question - not worth an SSRF
      surface in production.
  - date: '2026-09-02'
    status: active
    who: karolkow
    note: >
      B rewritten from scratch on the search branch instead of shipping
      separately: the same two hops already existed for A, so the reverse
      direction is one resolver, one hook and one summary row. Live sample of
      8 random accounts carrying `home_domain = lobstr.co`: 7 resolved to a
      name, so the row is worth showing. The transaction source-account
      surface followed the same day: the detail query already seeks the
      accounts row, so `home_domain` came along as one more column rather
      than the separate API task it looked like.
---

# FEATURE: SEP-2 federated addresses, both directions

## Two directions, deliberately separated

SEP-2 defines lookups in both directions and external feedback asked for both,
five weeks apart, using the same words for each. They are not one feature: the
cost differs by an order of magnitude.

| Scope | Query       | Direction                | Where                   | Cost                             |
| ----- | ----------- | ------------------------ | ----------------------- | -------------------------------- |
| **A** | `type=name` | `karol*lobstr.co` → `G…` | global search input     | browser only                     |
| **B** | `type=id`   | `G…` → `karol*lobstr.co` | account + tx source row | enrichment, storage, SSRF guards |

A shipped first and stands alone. B followed, and turned out to be the same
shape: the account's `home_domain` is already on the detail response, so the
browser can make both hops itself. The enrichment step, the stored column and
the server-side SSRF guards sketched below were **not built** — the request
never leaves the user's browser, so our infrastructure is not in the path.
That sketch is kept for the record, marked superseded.

## Scope A — resolve `name*domain` typed into search

Someone types `karol*lobstr.co` into the explorer's search box and lands on
that account's page.

### Why this is the cheap direction

The domain is typed by the user, in their own browser, and both hops are
public CORS-enabled GETs. Measured against the live network, 2026-09-01:

```
GET https://lobstr.co/.well-known/stellar.toml
  → FEDERATION_SERVER="https://lobstr.co/federation/"     ACAO: *
GET https://lobstr.co/federation/?q=test*lobstr.co&type=name
  → {"account_id":"GC526FUILJ6NLFXKCOOGTMDXNRW7MYSEK2UNRJV5FYWOGYDE4LOKXFEM"}
                                                          ACAO: *
```

So the browser can do the whole thing. No API route, no ClickHouse column, no
enrichment step, no server-side outbound fetch — and therefore none of scope
B's SSRF surface, because our infrastructure never issues the request.

### Where it hooks in

`web/src/search/directRouteFor.ts` is synchronous and stays that way — the
federation resolve is two network round-trips and cannot live in it. It hooks
into `web/src/pages/SearchResultsPage.tsx` instead, which is the single point
every entry path already converges on: the app-shell search bar and the home
hero both call `directRouteFor(q) ?? routes.search(q)`, and an input shaped
`name*domain` fails the classifier, so it arrives at `/search?q=…` on its own.
One hook, three entry points covered, neither caller touched.

### Acceptance criteria — scope A

- [x] `name*domain` typed into search resolves to the account page
- [x] **HTTPS only**, on both hops. A `FEDERATION_SERVER` that is not
      `https:` is refused, not fetched.
- [x] The returned `account_id` is shape-checked as a `G…` StrKey before
      any navigation. A federation server is operated by the domain owner
      and can return anything.
- [x] **A failed resolve says so.** Domain without a toml, toml without a
      `FEDERATION_SERVER`, server that 404s, CORS refusal, timeout — each
      surfaces an explicit message. An empty results page would read as
      "this address does not exist", which is a different and false claim.
- [x] Bounded: request timeout and a response-size cap (the cap was claimed
      here before it existed — `res.text()` read the whole body; added
      2026-09-02), so a hung host
      cannot leave the input spinning forever.
- [x] Never fires on ordinary queries — an input without exactly one `*`,
      or with an unparseable domain, must not trigger a network call.
- [x] **Docs updated** — N/A, no architectural surface changes (browser-only,
      no new endpoint, no schema change).
- [x] **API types regenerated** — N/A, does not touch `crates/api/**`.

### Implementation notes — scope A

Three files, frontend only:

- `web/src/search/federation.ts` (new) — classifier + resolve. Returns a
  result union, never throws; every failure path carries the sentence the UI
  shows.
- `web/src/pages/SearchResultsPage.tsx` — one `useQuery` + one effect. A
  federated input also short-circuits `useSearchResults` (passes `''`), so
  `/v1/search` is not called for a query it cannot answer.
- `web/src/search/GlobalSearchBar.tsx` — the header dropdown skips
  `/v1/search` for a federated input too. It knows nothing about SEP-2, so it
  answered zero hits and the dropdown read "No results for karol\*lobstr.co"
  while Enter went on to resolve that very address. It now says what Enter
  will do.
- `web/src/search/federation.test.ts`, `web/src/pages/SearchResultsPage.test.tsx`,
  `web/src/search/GlobalSearchBar.test.tsx` (new).

**Not given a federated path, deliberately:** the accounts list has no search
field at all (only a "With domain" toggle — StrKeys are opaque, so a prefix
filter is useless and exact lookup is the global search's job), and the
transactions list input is a `filter[...]` on a column, not a search.

### Design decisions — scope A

#### Emerged

1. **Hooked into the results page, not `directRouteFor`.** That file's own
   doc-comment warns against growing it, and it is synchronous — a two-hop
   resolve cannot live there. The results page is where all three entry paths
   already converge, so no caller changed.
2. **A federated query skips `/v1/search` entirely** rather than running it
   alongside. The backend knows nothing about SEP-2, so the request can only
   return zero hits, and an empty results table next to a resolve error reads
   as a contradiction.
3. **Failure is loud, in place of results, not beside them.** For a federated
   input the results card is replaced by the resolve's own state. "No results"
   would assert the address does not exist; the failure text asserts only what
   we know — which hop failed.
4. **Size cap is post-hoc, timeout is the real bound.** A streaming reader for
   a `stellar.toml` was not worth the code; marked `ponytail:` in the source
   with the upgrade path.
5. **Reused `isAccountId` from the UI lib** instead of a local regex, so the
   shape rule for "is this a Stellar account" stays in one place.

## Scope B — show the federated address on the account page

Where an account has a federated address (`name*domain.com`, SEP-2), show it
next to the raw `G…` StrKey on the account detail page and in the transaction
source-account row. Requires a reverse (`type=id`) federation lookup, cached as
an enrichment field — never resolved on the read path.

### Context

Nothing in the codebase performs any federation lookup. Two of the three
required pieces already exist:

- `accounts.home_domain` is indexed and stored
  (`crates/enrichment-worker/src/main.rs:377`); it is mutable via `SET_OPTIONS`,
  so the enrichment path already treats it as a moving value
  (`crates/enrichment-shared/src/enrich_and_persist/sep1_assets.rs:79-99`).
- A `stellar.toml` fetcher exists from the SEP-1 asset-details work
  (task 0188, `crates/enrichment-shared/src/sep1/`).

Missing: reading `FEDERATION_SERVER` out of the fetched toml and issuing the
reverse query.

### Implementation sketch

1. Extend the SEP-1 toml parse to capture `FEDERATION_SERVER`.
2. New enrichment step: for accounts with a non-empty `home_domain`, call
   `GET <federation_server>?q=<G-strkey>&type=id`, store `stellar_address`.
3. Surface the field on the account detail DTO + transaction source-account row.
4. Frontend: render `name*domain.com` above the truncated StrKey; StrKey stays
   the copyable canonical value.

### Constraints / risks

- **Off the read path.** The lookup hits a third-party HTTP server. It must run
  in enrichment with a cached result + TTL, never during a page request.
- **Low coverage.** Only accounts that set `home_domain` can ever resolve;
  in practice mostly exchanges and custodial wallets.
- **Not authoritative.** A federation server is operated by the domain owner and
  can return anything. Display it as a claim of that domain, never as a verified
  identity, and never let it replace the StrKey as the copyable value.
- Needs a negative cache — most lookups will 404 and must not be retried hot.
- **`FEDERATION_SERVER` is a URL an untrusted third party chose.** We fetch it
  because an account set a home domain, so the destination is attacker-supplied
  in the ordinary case, not the exotic one. Whoever writes this must treat the
  fetch as SSRF-shaped from the start — retrofitting the guards after the first
  fetch works is how this class of bug ships. The concrete requirements are in
  the acceptance criteria rather than here, so they cannot be read as advice.

### Coverage, measured before building — 2026-09-01

The task's own note said to scope coverage first and close the direction if
the intersection was negligible. Sample: 100 accounts carrying a non-empty
`home_domain`, taken from `/v1/accounts?filter[with_domain]=true`.

| Step                                                           | Count                   |
| -------------------------------------------------------------- | ----------------------- |
| accounts with a home domain (sample)                           | 100                     |
| …whose domain serves a `stellar.toml` with `FEDERATION_SERVER` | 85 (5 distinct domains) |
| …that the server actually reverse-resolves to a name           | **33**                  |

Not negligible, so it was built. The 52 that fail are accounts on a
federation-capable domain that simply have no registered name — the server
answers `Not found`, which is why the row is absent rather than errored.

### Implementation notes — scope B

- `web/src/search/federation.ts` — `resolveFederatedName(accountId, homeDomain)`.
  Shares the `stellar.toml → FEDERATION_SERVER` hop with scope A via
  `federationServerFor`; only the query direction differs.
- `web/src/pages/accounts/AccountSummary.tsx` — one `useQuery`, one summary
  row, rendered only when the lookup returns a name.
- 6 further tests. Verified on the running app against two real mainnet
  accounts: one that resolves (row present, name matches the federation
  server's own answer) and one on the same domain that does not (no row).

### Design decisions — scope B

#### Emerged

6. **Both sides must agree.** The returned `stellar_address` is rejected
   unless it ends in `*<home_domain>`. The account names the domain on-ledger
   and the domain names the account back inside its own namespace; without
   the suffix check a domain could claim an account into someone else's.
7. **Not stored, resolved per view.** The value belongs to the domain and can
   change at any moment; a cached copy would present a stale claim as a
   current one. React Query's window is the only caching.
8. **Silent when absent, unlike scope A.** Nobody asked a question here — the
   row is an attribute the account may not have, so its absence asserts
   nothing. In search the user did ask, so silence there would have answered
   wrongly.
9. **Never the copyable value, never a link.** The StrKey stays canonical and
   copyable; the address renders as plain text with a title explaining it is
   the domain's claim. One real name in the sample is literally
   `http://…*lobstr.co`, which is exactly why it is not linkified.

### Acceptance criteria — scope B

- [x] `FEDERATION_SERVER` parsed from `stellar.toml`
- [x] ~~**The URL is validated before it is fetched:**~~ **N/A — superseded.**
      No server-side fetch exists. The browser fetches, from the user's own
      machine, a domain the account itself published. The three criteria below
      guard an enrichment worker that was never built; they return the moment
      any of this moves server-side.
      _(original text)_ HTTPS only, and the
      resolved address rejected when it is loopback, link-local, RFC1918 or a
      cloud metadata endpoint. Check after DNS resolution, not on the hostname —
      a hostname resolving to `169.254.169.254` passes any string check.
- [ ] ~~**Every redirect target re-validated the same way**, and the redirect
      chain bounded — one allowed hop is enough to reach the metadata service.\*\*~~ N/A, see above.
- [x] Connect and read timeouts and a response-size cap — kept, they bound the
      browser's own request (`AbortSignal.timeout`, size check in `getBounded`)
- [x] Reverse (`type=id`) lookup runs — in the browser, not in enrichment; no
      TTL or negative cache needed because nothing is persisted
- [x] ~~Resolved address exposed on the account DTO~~ N/A — `home_domain` is
      already on the DTO and the browser does the rest, so no API change.
      Transaction source-account row: **not done**, account detail only
- [x] Frontend renders it as secondary to the StrKey, not as a replacement
- [x] Absent / failed lookup degrades silently to StrKey-only
- [x] **Docs updated** — N/A. The enrichment step the original design would
      have added does not exist; nothing in `docs/architecture/**` describes
      browser-side display logic.
- [x] **API types regenerated** — N/A, no backend change.

## Measured limitation — federation servers that omit CORS

SEP-2 requires `Access-Control-Allow-Origin: *` on **every** federation
response. Not every server obeys, and a browser-only implementation simply
gets nothing from those: the request is blocked before our code sees it, so
the address silently has no name.

Measured 2026-09-02 with real GET requests (a HEAD probe is useless — several
of these reject HEAD):

| Domain              | Federation server                     | CORS                |
| ------------------- | ------------------------------------- | ------------------- |
| `lobstr.co`         | `lobstr.co/federation/`               | yes, on 200 and 404 |
| `staging.lobstr.co` | `staging.lobstr.co/federation/`       | yes                 |
| `sl8.online`        | `stellar.sl8.online/sep2`             | yes                 |
| `stellarterm.com`   | `fed.stellarterm.com/federation/`     | **no**              |
| `sanbeban.com`      | `wallet.sanbeban.com/api/federation/` | **no**              |
| `bitgo.com`         | `www.bitgo.com/api/v2/xlm/federation` | no response at all  |

### The tail, measured — it is not blocked, it simply does not federate

The first pass stopped at the six biggest domains and left ~12% of accounts
unexamined, which made the gap look like "84% works, 16% is broken". Measured
2026-09-03 over the next 115 domains by account count (59 450 accounts, 46% of
that remainder), two live requests each:

| Result                                             | Domains | Accounts | Share of sample |
| -------------------------------------------------- | ------- | -------- | --------------- |
| No `stellar.toml` at all                           | 82      | 33 967   | 57.1%           |
| `stellar.toml` present, **no `FEDERATION_SERVER`** | 31      | 24 687   | 41.5%           |
| Working, sends the header                          | 2       | 796      | 1.3%            |
| **Omits the header**                               | **0**   | **0**    | **0%**          |

Not one CORS violator in the tail. Those domains are not blocked — 98.7% of
the sampled accounts sit on a domain that runs no federation server, so there
is no name to show whatever we do at our end.

So the honest arithmetic, against the 1 055 369 accounts that declare a
`home_domain`:

|                                                                       | Accounts | Share    |
| --------------------------------------------------------------------- | -------- | -------- |
| Works today                                                           | 881 118  | 83.5%    |
| **Lost to a missing CORS header** (`stellarterm.com`, `sanbeban.com`) | ~19 955  | **1.9%** |
| `bitgo.com` — no response at all, CORS moot                           | 25 366   | 2.4%     |
| No federation server (measured + tail extrapolation)                  | ~128 000 | ~12.1%   |

**The prize for defeating CORS is 1.9% of accounts, not 16%.** Record that
before anyone reaches for a proxy.

### Can the requirement be bypassed?

Technically yes: CORS binds browsers, not servers, and this repo already has a
hardened SEP-1 fetcher (`crates/enrichment-shared/src/sep1/`) with a body cap,
HTTPS enforced across redirects and a real TOML parser. It would work. It also
brings back exactly the surface this scope was designed to avoid, and the two
directions differ:

- **Forward (search).** The domain is _typed by whoever is using the site_.
  Proxying means our production API connects to an arbitrary address supplied
  by an anonymous visitor — textbook SSRF (`a*169.254.169.254` and the cloud
  metadata endpoint). An allowlist defends it, but then only listed domains
  resolve, which is not the tail anyway. **Keep this in the browser
  permanently.**
- **Reverse (account / transaction pages).** The domain comes from the ledger,
  and the enrichment path already walks those same domains for SEP-1. The
  marginal new surface is small.

**Decision: not now.** 1.9% does not buy a second fetch path plus an SSRF
surface in production. If it is ever revisited, do the reverse direction first
through the existing enrichment (those requests already leave our
infrastructure) and never move the forward direction server-side.

## Future work

- ~~The transaction source-account row still shows the bare StrKey.~~ **Done
  2026-09-02.** The API field turned out to be nearly free: the detail path
  already key-seeks `accounts` for the source StrKey, so `home_domain` rides
  the same seek as `source_account_home_domain`. It needs `argMax(home_domain,
last_seen_ledger)` rather than the shared `resolve_accounts` helper — that
  one dedups ReplacingMergeTree versions with `LIMIT 1 BY id`, exact only for
  columns immutable across versions, which `home_domain` is not. Measured on a
  `GA22%` slice: 195 of 3603 accounts carry more than one version, none with a
  differing `home_domain` today — so the wrong dedup would not bite yet, but
  it is one domain change away from doing so.

## Notes

~~Worth scoping the coverage before building: count accounts with a non-empty
`home_domain`, then sample how many of those domains actually serve a
`FEDERATION_SERVER`. If the intersection is negligible, close this instead.~~
Done — see _Coverage, measured before building_ above. It was not negligible.
