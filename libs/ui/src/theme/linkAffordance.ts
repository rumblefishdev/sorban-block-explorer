import { alpha, type Theme } from '@mui/material/styles';

/**
 * The one definition of "this text is a link" for links **in content** — an
 * identifier, an asset code inside a table cell, anything the reader would
 * otherwise take for static text (task 0535).
 *
 * Navigation and chrome are deliberately NOT in scope: the nav, footer, nav
 * buttons, breadcrumbs and chips keep `textDecoration: 'none'`, because their
 * position or container already says they are interactive. Underlining those
 * adds noise without adding information.
 *
 * Before this existed, a link in content was drawn exactly like body text —
 * same colour, no decoration — so the only signal was `cursor: pointer`: you
 * had to hover to find out, and on touch there was nothing at all.
 *
 * Two details are load-bearing rather than cosmetic:
 *
 * - **The underline is muted, not full strength.** Every hash in the
 *   transactions list is a link; a solid rule under each row reads as noise
 *   instead of affordance. Hover promotes it to `currentColor`.
 * - **The offset clears the descenders.** Identifiers are monospace and full of
 *   `g`, `y`, `p`, which a default underline cuts straight through.
 *
 * Colour is NOT part of this: it stays a hierarchy tool. The "value moved" cell
 * grades amount / code / count as primary / secondary / tertiary, and forcing a
 * link colour there would invert what the cell is saying.
 */
export function contentLinkSx(theme: Theme) {
  return {
    textDecoration: 'underline',
    textDecorationColor: alpha(theme.palette.text.primary, 0.35),
    textUnderlineOffset: '3px',
    '&:hover': { textDecorationColor: 'currentColor' },
  } as const;
}
