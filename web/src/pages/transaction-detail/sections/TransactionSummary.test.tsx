import type {
  E3ResponseTransactionDetailLight,
  XdrOperationDto,
} from '@rumblefish/api-types';
import { screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { renderWithProviders } from '../../../test-utils.js';

import { opFailReason, TransactionSummary } from './TransactionSummary.js';

function op(
  op_type: string,
  application_order: number,
  result_code: string | null
): XdrOperationDto {
  return { op_type, application_order, details: {}, result_code };
}

function tx(
  operations: XdrOperationDto[],
  successful = false
): E3ResponseTransactionDetailLight {
  return {
    hash: 'ab'.repeat(32),
    ledger_sequence: 1,
    created_at: '2026-01-01T00:00:00Z',
    fee_charged: 100,
    successful,
    operation_count: operations.length,
    operations: [],
    heavy: {
      memo_type: null,
      memo: null,
      signatures: [],
      fee_bump_source: null,
      envelope_xdr: null,
      result_xdr: null,
      diagnostic_events: [],
      contract_events: [],
      operations,
      result_code: 'TxFailed',
      operation_tree: null,
    },
    heavy_fields_status: 'ok',
  } as unknown as E3ResponseTransactionDetailLight;
}

describe('opFailReason', () => {
  it('names the first failing op with its code (0352 fixture 7af6d0ed…)', () => {
    const reason = opFailReason(
      tx([
        op('begin_sponsoring_future_reserves', 1, 'Success'),
        op('create_account', 2, 'LowReserve'),
        op('create_account', 3, 'OpNoAccount'),
      ])
    );
    expect(reason).toBe('Create Account #2 — LOW_RESERVE (+1 more failed)');
  });

  it('returns null when codes are absent (old responses, validation failures)', () => {
    expect(opFailReason(tx([op('create_account', 1, null)]))).toBeNull();
    expect(opFailReason(tx([]))).toBeNull();
  });
});

describe('TransactionSummary failed strip', () => {
  it('shows the per-op reason on the strip', () => {
    renderWithProviders(
      <>
        <TransactionSummary tx={tx([op('create_account', 1, 'LowReserve')])} />
      </>
    );
    expect(screen.getByText(/Create Account #1 — LOW_RESERVE/)).toBeTruthy();
  });
});

/**
 * Memo, fee source and the fail reason all live in the archive-fetched block.
 * With it absent they must say so rather than render the same dash a real
 * "has none" would produce, or — for the reason — fall silent, which reads as
 * the validation-failure class rather than "we could not read it" (0377 F1).
 */
describe('TransactionSummary with no archive data', () => {
  function heavyless(successful: boolean) {
    return {
      ...tx([], successful),
      heavy: null,
      inner_tx_hash: 'ab'.repeat(32),
    } as unknown as E3ResponseTransactionDetailLight;
  }

  it('marks the memo unavailable instead of dashing it', () => {
    renderWithProviders(
      <>
        <TransactionSummary tx={heavyless(true)} />
      </>
    );

    // Two cells resolve to the marker: memo and, via inner_tx_hash, fee source.
    expect(screen.getAllByText('Not available').length).toBeGreaterThanOrEqual(
      2
    );
  });

  it('says the fail reason is unavailable rather than implying a validation failure', () => {
    renderWithProviders(
      <>
        <TransactionSummary tx={heavyless(false)} />
      </>
    );

    expect(screen.getByText(/reason unavailable/)).toBeTruthy();
  });

  it('keeps a real memo-less transaction on the plain dash', () => {
    renderWithProviders(
      <>
        <TransactionSummary tx={tx([], true)} />
      </>
    );

    expect(screen.queryByText('Not available')).toBeNull();
  });
});
