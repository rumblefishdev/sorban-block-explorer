import { keepPreviousData } from '@tanstack/react-query';

/**
 * Cadence knob for the poll target (`1.5 × cadence` after the last close).
 * Observed mainnet close interval is ~5.8s, but this is set deliberately
 * *below* that (5.5s) to bias each fetch slightly EARLY in the next gap.
 * The two 1:1 failure modes are asymmetric: firing late catches 2 blocks
 * in one fetch (breaks 1:1, unrecoverable), while firing early just yields
 * an empty fetch that the cheap floor retry recovers. Leaning early trades
 * the costly failure for the cheap one. Not pushed lower than this or
 * almost every fetch would be a wasted empty+retry (over-fetch).
 */
const LEDGER_CADENCE_MS = 5_500;
/** Floor between polls — also the recheck step when a block runs late. */
const LIVE_MIN_MS = 4_000;
/** Cap (= 1.5 × cadence) so a fresh-block / stalled fetch still rechecks. */
const LIVE_MAX_MS = 8_250;

/**
 * Policy for every per-ledger live poll: the home activity feeds
 * (ledgers + transactions) and `useNetworkStats` (KPIs, TopNav, LIVE
 * badge). `staleTime` sits at the poll floor so any focus/mount trigger
 * between ticks also refetches fresh. The cadence itself is the adaptive
 * `midpointPollDelay` below — set it per hook as `refetchInterval`, since
 * each query reads its newest timestamp from a different field. The
 * server side cooperates: these endpoints ship `Cache-Control:
 * max-age=0` so every tick actually consults the origin (a browser-cache
 * TTL ≥ the cadence would re-serve a stale payload and the feed would
 * batch 2-3 ledgers per visible update), and the stats moka cache TTL
 * sits below the cadence for the same reason.
 */
export const livePolicy = {
  staleTime: LIVE_MIN_MS,
} as const;

/**
 * Adaptive poll delay that aims each fetch at the MIDPOINT of the next
 * inter-block gap, given the close timestamp of the newest block seen.
 *
 * Target = lastClose + 1.5 × cadence: one cadence to reach the next block,
 * plus half a cadence to land in the centre of its interval — the point
 * that maximises the symmetric margin against ledger-close jitter, so it
 * minimises both empty fetches (0 new blocks) and double catches (2 blocks
 * in one fetch). Latency is deliberately traded away for that margin.
 *
 * `delay = 1.5 × cadence − elapsedSinceClose`, clamped to [MIN, MAX]. The
 * subtraction self-corrects with no stored state: a late block keeps the
 * same `closedAt` while wall-clock advances, so `elapsed` grows, the delay
 * shrinks toward MIN, and the feed rechecks sooner until the block lands
 * and `closedAt` jumps — re-anchoring the phase on the real ledger clock.
 * Uses the client clock for "now"; sub-second skew is irrelevant at this
 * cadence and latency is not a goal. This only *approaches* a clean 1:1 —
 * with non-zero close jitter some 0/2 fetches are unavoidable (a client
 * cannot know future arrivals); a true 1:1 needs server push (SSE/WS),
 * out of scope on the Lambda-only backend.
 */
export function midpointPollDelay(closedAt: string | undefined): number {
  if (!closedAt) return LEDGER_CADENCE_MS;
  const close = Date.parse(closedAt);
  if (Number.isNaN(close)) return LEDGER_CADENCE_MS;
  const delay = 1.5 * LEDGER_CADENCE_MS - (Date.now() - close);
  return Math.min(LIVE_MAX_MS, Math.max(LIVE_MIN_MS, delay));
}

/**
 * Rows per page for every cursor-paginated list/section. Also the loading
 * skeleton height, so a full page and its placeholder stay the same size.
 */
export const PAGE_SIZE = 20;

/**
 * Default policy for cursor-paginated list queries. `placeholderData:
 * keepPreviousData` keeps the previous page's rows on screen while the
 * next cursor is being fetched, so Next clicks don't flash a spinner
 * between pages.
 */
export const listPolicy = {
  staleTime: 60_000,
  placeholderData: keepPreviousData,
} as const;

/**
 * Default policy for detail queries with embedded cursor-paginated
 * sub-sections (e.g. ledger transactions). Same `placeholderData` rule
 * as `listPolicy` for the embedded list.
 */
export const detailPolicy = {
  staleTime: 5 * 60_000,
  placeholderData: keepPreviousData,
} as const;

export const searchPolicy = {
  staleTime: 0,
  gcTime: 0,
} as const;

/**
 * Policy for the two SEP-2 federation lookups (task 0443). They differ from
 * every other query here: the host is a third party the account or the user
 * named, not our API.
 *
 * - `retry: false` — a federation server that says "not found" means it, and
 *   a hostile or dead one must not be dialled twice.
 * - `refetchOnWindowFocus: false` — the global default is `true`, which would
 *   re-run both hops against someone else's server every time the tab regains
 *   focus, for a value that changes on a human timescale.
 * - No caching at all, matching `searchPolicy`. SEP-2 is explicit: "Federation
 *   responses should not be cached. Some organizations may generate random IDs
 *   to protect their users' privacy. Those IDs may change over time." A cached
 *   forward answer could send someone to an account the anchor has since
 *   rotated away from, which is the one failure worth paying an extra request
 *   to avoid.
 */
export const federationPolicy = {
  staleTime: 0,
  gcTime: 0,
  retry: false,
  refetchOnWindowFocus: false,
} as const;
