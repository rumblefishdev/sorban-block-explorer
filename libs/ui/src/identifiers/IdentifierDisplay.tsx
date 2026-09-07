import { useMemo } from 'react';

import Box from '@mui/material/Box';
import type { SxProps, Theme } from '@mui/material/styles';

import { formatInteger } from '../format/index.js';
import { contentLinkSx } from '../theme/linkAffordance.js';
import { monoFontFamily, secondaryFontFamily } from '../theme/typography.js';

import { useLinkComponent } from './LinkComponentContext.js';
import { getIdentifierHref } from './routes.js';
import { getDefaultTruncation, truncateMiddle } from './truncate.js';
import type { EntityType, TruncationConfig } from './types.js';

function makeIdentifierSx(
  linked: boolean,
  fullWidth: boolean,
  tone: 'default' | 'inherit',
  fontSize: number | string,
  mono: boolean
): SxProps<Theme> {
  const inheritColor = tone === 'inherit';
  return (theme: Theme) => ({
    fontFamily: mono ? monoFontFamily : secondaryFontFamily,
    fontSize,
    fontWeight: 500,
    lineHeight: 1.4,
    color: inheritColor ? 'inherit' : theme.palette.text.primary,
    '&:visited': {
      color: inheritColor ? 'inherit' : theme.palette.text.primary,
    },
    // The underline that says "this is a link" comes from `contentLinkSx`, the
    // one definition shared with every other in-content link (task 0535).
    // Gated on `linked`, because that is exactly the distinction it has to
    // carry: `linked={false}` renders next to linked identifiers on the same
    // screen (`ContractSummary`), and before this they were identical.
    //
    // `tone: 'inherit'` sits on coloured surfaces where `text.primary` is the
    // wrong reference, so it keeps the inherited `currentColor`.
    textDecoration: 'none',
    ...(linked && {
      ...contentLinkSx(theme),
      ...(inheritColor && { textDecorationColor: 'currentColor' }),
    }),
    display: 'inline-flex',
    alignItems: 'center',
    maxWidth: fullWidth ? '100%' : undefined,
    overflow: fullWidth ? 'hidden' : 'visible',
    textOverflow: fullWidth ? 'ellipsis' : 'clip',
    whiteSpace: 'nowrap',
    cursor: linked ? 'pointer' : 'inherit',
    '&:hover': linked
      ? {
          textDecorationColor: 'currentColor',
          ...(inheritColor
            ? {}
            : { color: theme.palette.surface.primaryMainAlt }),
        }
      : undefined,
    '&:focus-visible': {
      outline: inheritColor
        ? '2px solid currentColor'
        : `2px solid ${theme.palette.stroke.action}`,
      outlineOffset: 2,
      borderRadius: 2,
    },
  });
}

export interface IdentifierDisplayProps {
  value: string;
  type: EntityType;
  truncate?: boolean;
  truncation?: TruncationConfig;
  linked?: boolean;
  href?: string;
  /**
   * 'inherit' makes the link adopt the surrounding text colour — needed when
   * rendered on coloured backgrounds (e.g. flow-tree node cards). Defaults to
   * 'default' (theme text colour).
   */
  tone?: 'default' | 'inherit';
  /**
   * Font size of the rendered identifier. Defaults to 14. Pass `'inherit'`
   * when rendered inline inside smaller surrounding text (e.g. a reserves
   * cell) so the identifier matches the adjacent value rather than forcing
   * its own size.
   */
  fontSize?: number | string;
  /**
   * Force the monospace font on/off, overriding the type-driven default. Needed
   * when an entity's `type` doesn't match the value shape — e.g. a ledger's
   * HASH cell uses `type="ledger"` (no link/route) but must render mono like
   * every other hash so the truncated `XXXX…XXXX` is fixed-width (copy buttons
   * line up). Defaults to the per-type heuristic.
   */
  mono?: boolean;
  className?: string;
  'aria-label'?: string;
}

function formatForDisplay(type: EntityType, value: string): string {
  if (type === 'ledger' && /^\d+$/.test(value)) {
    return formatInteger(Number(value));
  }
  return value;
}

export function IdentifierDisplay({
  value,
  type,
  truncate = true,
  truncation,
  linked = true,
  href,
  tone = 'default',
  fontSize = 14,
  mono,
  className,
  'aria-label': ariaLabel,
}: IdentifierDisplayProps) {
  // Internal links render via the app-provided router link (client-side SPA
  // nav); falls back to a native `<a>` when no provider is present. A bare
  // `<a>` would hard-reload the whole app on every list→detail click. lore-0384.
  const LinkComponent = useLinkComponent();
  // Type-driven font: opaque identifiers (hashes, addresses, contract / tx /
  // pool ids) read in the mono font where fixed width aids scanning. Asset
  // "ids" are human ticker codes (USDC, AQUA) and ledger ids are plain
  // sequence numbers — both read in the body font like the value beside
  // them (matches Figma). Ledger stays a link, just not mono — but a ledger
  // HASH cell overrides via `mono` (see prop doc).
  const isMono = mono ?? (type !== 'asset' && type !== 'ledger');
  const cfg = truncation ?? getDefaultTruncation(type);
  const formatted = formatForDisplay(type, value);
  const displayText = truncate ? truncateMiddle(formatted, cfg) : formatted;
  const sx = useMemo(
    () => makeIdentifierSx(linked, !truncate, tone, fontSize, isMono),
    [linked, truncate, tone, fontSize, isMono]
  );

  // NFT identity is composite `(contract_id, token_id)`; pass `href`
  // explicitly when `type='nft'` (no production callsite does today).
  // `getIdentifierHref` throws on `'nft'` if reached without an
  // override — safety net, not hot path.
  return (
    <Box
      component={linked ? LinkComponent : 'span'}
      {...(linked && { href: href ?? getIdentifierHref(type, value) })}
      className={className}
      aria-label={ariaLabel ?? value}
      sx={sx}
    >
      {displayText}
    </Box>
  );
}
