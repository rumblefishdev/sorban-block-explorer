import { ThemeProvider } from '@mui/material/styles';
import { render, screen } from '@testing-library/react';
import type { ReactNode } from 'react';
import { describe, expect, it } from 'vitest';

import { createExplorerTheme } from '../theme/theme.js';

import { IdentifierDisplay } from './IdentifierDisplay.js';

const ACCOUNT = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN';

function withTheme(ui: ReactNode, mode: 'light' | 'dark' = 'dark') {
  return <ThemeProvider theme={createExplorerTheme(mode)}>{ui}</ThemeProvider>;
}

/**
 * Task 0535. The underline is the ONLY thing that separates a link from static
 * text here — colour and weight are identical by design, because colour stays a
 * hierarchy tool. `linked={false}` renders beside linked identifiers on the same
 * screen (`ContractSummary`), so if this ever regresses the two become
 * indistinguishable at rest and the difference is discoverable only by hovering
 * — and not at all on touch.
 */
describe('IdentifierDisplay link affordance', () => {
  it('underlines a linked identifier', () => {
    render(withTheme(<IdentifierDisplay value={ACCOUNT} type="account" />));
    const el = screen.getByRole('link');
    // jsdom does not expand the shorthand into `textDecorationLine`.
    expect(getComputedStyle(el).textDecoration).toBe('underline');
  });

  it('leaves an unlinked identifier undecorated', () => {
    render(
      withTheme(
        <IdentifierDisplay value={ACCOUNT} type="account" linked={false} />
      )
    );
    // No role=link: it renders as a span, so query by its aria-label.
    const el = screen.getByLabelText(ACCOUNT);
    expect(getComputedStyle(el).textDecoration).toBe('none');
  });

  it('keeps the underline muted at rest so dense tables stay readable', () => {
    render(withTheme(<IdentifierDisplay value={ACCOUNT} type="account" />));
    const style = getComputedStyle(screen.getByRole('link'));
    // Muted, not `currentColor` — a solid rule under every row of the
    // transactions list reads as noise rather than as affordance.
    expect(style.textDecorationColor).toMatch(/rgba\(.+0\.35\)/);
    expect(style.textUnderlineOffset).toBe('3px');
  });

  it('applies in light mode too', () => {
    render(
      withTheme(<IdentifierDisplay value={ACCOUNT} type="account" />, 'light')
    );
    expect(getComputedStyle(screen.getByRole('link')).textDecoration).toBe(
      'underline'
    );
  });
});
