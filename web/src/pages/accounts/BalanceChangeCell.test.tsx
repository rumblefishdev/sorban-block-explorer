import type { AccountBalanceChange } from '@rumblefish/api-types';
import { screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { renderWithProviders } from '../../test-utils.js';

import { BalanceChangeCell } from './BalanceChangeCell.js';

const USDC = 'USDC-GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN';
const NFT_CONTRACT = 'CBHUX3RSBKAL7MJUOUA3PPW3TA65YRFLCGASJ5TNHY7HDLXCHWZQA6GR';

function change(
  over: Partial<AccountBalanceChange> = {}
): AccountBalanceChange {
  return {
    asset: USDC,
    asset_code: 'USDC',
    decimals: 7,
    amount: '60970653780',
    nft_delta: 0,
    token_id: null,
    ...over,
  };
}

describe('BalanceChangeCell', () => {
  it('says "not indexed" below the floor instead of drawing a zero', () => {
    // `null` means the transfer index does not cover this transaction — a gap
    // in what we have looked at, not a measurement that nothing moved. Drawing
    // it as `0` would put a figure on screen nobody measured.
    renderWithProviders(<BalanceChangeCell changes={null} />);

    expect(screen.getByText('Not indexed')).toBeInTheDocument();
    expect(screen.queryByText('0')).not.toBeInTheDocument();
  });

  it('draws a MEASURED zero when the account came out level', () => {
    // The counterpart: indexed, and this account's balances did not change —
    // an offer placed, a round-trip that netted out, a payment to self.
    renderWithProviders(<BalanceChangeCell changes={[]} />);

    expect(screen.getByText('0')).toBeInTheDocument();
    expect(screen.queryByText('Not indexed')).not.toBeInTheDocument();
  });

  it('signs the amount, scales it, and keeps US grouping', () => {
    renderWithProviders(<BalanceChangeCell changes={[change()]} />);

    expect(screen.getByText('+6,097.065378')).toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'USDC' })).toBeInTheDocument();
  });

  it('marks money out with a minus, not a bare number', () => {
    renderWithProviders(
      <BalanceChangeCell changes={[change({ amount: '-13499' })]} />
    );

    expect(screen.getByText('−0.0013499')).toBeInTheDocument();
  });

  it('links a named piece to that NFT, not to a filtered list', () => {
    // The whole point of resolving the token id: a list with a 56-character
    // contract typed into its search box is not where someone clicking an NFT
    // wants to land.
    renderWithProviders(
      <BalanceChangeCell
        changes={[
          change({
            asset: NFT_CONTRACT,
            asset_code: 'TALKMP25',
            amount: null,
            nft_delta: 1,
            token_id: '44',
          }),
        ]}
      />
    );

    expect(screen.getByText('+1 NFT #44')).toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'TALKMP25' })).toHaveAttribute(
      'href',
      `/nfts/${NFT_CONTRACT}/44`
    );
  });

  it('lists every piece of a bulk move separately, each with its own link', () => {
    // A transaction moving three pieces arrives as three entries, one per
    // piece. The cell shows the first and collapses the rest into `+N`; the
    // tooltip carries all of them, each linking to its own NFT — so no piece
    // is reachable only as part of a count.
    renderWithProviders(
      <BalanceChangeCell
        changes={['101', '102', '103'].map((token_id) =>
          change({
            asset: NFT_CONTRACT,
            asset_code: 'TALKMP25',
            amount: null,
            nft_delta: 1,
            token_id,
          })
        )}
      />
    );

    expect(screen.getByText('+1 NFT #101')).toBeInTheDocument();
    expect(screen.getByText('+2')).toBeInTheDocument();
  });

  it('falls back to the collection when the piece cannot be named', () => {
    // Several pieces at once, or a collection whose ownership rows are not
    // indexed. A coarser destination beats a confident wrong one — and it must
    // never be `/assets/C…`, which 404s: an NFT collection has no `assets` row,
    // which is why the read joins that table LEFT.
    renderWithProviders(
      <BalanceChangeCell
        changes={[
          change({
            asset: NFT_CONTRACT,
            asset_code: 'TALKMP25',
            amount: null,
            nft_delta: 3,
            token_id: null,
          }),
        ]}
      />
    );

    expect(screen.getByText('+3 NFT')).toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'TALKMP25' })).toHaveAttribute(
      'href',
      `/nfts?contract=${NFT_CONTRACT}`
    );
  });

  it('counts a non-fungible movement as pieces, never as zero', () => {
    // `amount: null` is a movement with no amount BY NATURE (the event carries
    // a token id, not a value). The owner changed; a `0` would say otherwise.
    renderWithProviders(
      <BalanceChangeCell
        changes={[
          change({
            asset: NFT_CONTRACT,
            asset_code: 'TALKMP25',
            amount: null,
            nft_delta: -1,
          }),
        ]}
      />
    );

    expect(screen.getByText('−1 NFT')).toBeInTheDocument();
    expect(screen.queryByText('0')).not.toBeInTheDocument();
  });

  it('shows the asset that moved FIRST and collapses the rest into a count', () => {
    // Order comes from the API, which returns each transaction's assets in the
    // order their movements occurred on the chain. The cell renders it as
    // given and must not re-sort — there are no prices here, so any ranking by
    // amount would compare quantities that are not comparable.
    renderWithProviders(
      <BalanceChangeCell
        changes={[
          change({ asset: 'native', asset_code: null, amount: '5' }),
          change(),
        ]}
      />
    );

    expect(screen.getByText('+0.0000005')).toBeInTheDocument();
    expect(screen.getByText('+1')).toBeInTheDocument();
    expect(screen.queryByText('+6,097.065378')).not.toBeInTheDocument();
  });

  it('prints an unlinkable asset as text, never as a link that 404s', () => {
    // The API empties `asset` when `/assets/{id}` cannot answer — a token with
    // no `assets` row. Its contract StrKey exists, so a naive cell would build
    // a live-looking link to a page that returns 404.
    renderWithProviders(
      <BalanceChangeCell
        changes={[change({ asset: '', asset_code: 'SOMETOKEN', amount: '5' })]}
      />
    );

    expect(screen.getByText('SOMETOKEN')).toBeInTheDocument();
    expect(screen.queryByRole('link')).not.toBeInTheDocument();
  });

  it('names an unregistered token rather than borrowing XLM’s ticker', () => {
    // A bespoke token with no on-chain symbol has no code at all. Falling
    // through to the native label would put someone else's asset on screen.
    renderWithProviders(
      <BalanceChangeCell
        changes={[change({ asset: '', asset_code: null, amount: '1' })]}
      />
    );

    expect(screen.getByText('Unnamed token')).toBeInTheDocument();
    expect(screen.queryByText('XLM')).not.toBeInTheDocument();
    expect(screen.queryByRole('link')).not.toBeInTheDocument();
  });
});
