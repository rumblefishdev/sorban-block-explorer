import type { E3ResponseTransactionDetailLight } from '@rumblefish/api-types';
import ErrorOutlineIcon from '@mui/icons-material/ErrorOutline';
import { Box, Stack, Typography } from '@mui/material';
import {
  Chip,
  Dash,
  IdentifierDisplay,
  IdentifierWithCopy,
  RelativeTimestamp,
  StatusChip,
} from '@rumblefish/soroban-block-explorer-ui';

import { FeeCell } from '../../detail/FeeCell.js';
import { SectionCard } from '../../detail/SectionCard.js';
import { SummaryRow } from '../../detail/SummaryRow.js';
import { formatAbsoluteUtc } from '../../transactions/formatters.js';
import { formatOperationType } from '../../transactions/operationTypes.js';
import { describeMemo } from '../shared/describeMemo.js';
import { UnavailableValue } from '../shared/Unavailable.js';

interface TransactionSummaryProps {
  tx: E3ResponseTransactionDetailLight;
}

/** `"LowReserve"` → `"LOW_RESERVE"` — Stellar's canonical constant style for
 *  result codes; a pure case transform of the XDR library's variant name. */
function screamingSnake(code: string): string {
  return code.replace(/([a-z0-9])([A-Z])/g, '$1_$2').toUpperCase();
}

/** The fail reason from per-op result codes (task 0352): the FIRST failing
 *  operation named with its code, e.g. `Create Account #2 — LOW_RESERVE`.
 *  A transaction fails when ANY op fails, so later failures are counted, not
 *  hidden. Null for validation-level failures (`TxBadSeq`, …), where no
 *  operation was ever attempted and the tx-level code is the whole truth. */
export function opFailReason(
  tx: E3ResponseTransactionDetailLight
): string | null {
  const failed = (tx.heavy?.operations ?? []).filter(
    (op) => op.result_code != null && op.result_code !== 'Success'
  );
  const first = failed[0];
  if (first?.result_code == null) return null;
  const label = formatOperationType(first.op_type.toUpperCase());
  const more = failed.length - 1;
  return `${label} #${first.application_order} — ${screamingSnake(
    first.result_code
  )}${more > 0 ? ` (+${more} more failed)` : ''}`;
}

function MemoCell({
  memoType,
  memo,
  unavailable,
}: {
  memoType: string | null | undefined;
  memo: string | null | undefined;
  unavailable: boolean;
}) {
  // A memo can carry an exchange deposit id, so "no memo" is load-bearing and
  // must not stand in for "we never fetched the memo".
  if (unavailable) return <UnavailableValue />;
  const { typeLabel, content } = describeMemo(memoType, memo);
  if (typeLabel == null) return <Dash />;
  return (
    <Stack direction="row" spacing={1} alignItems="center" sx={{ minWidth: 0 }}>
      <Typography
        variant="bodySmRegular"
        sx={(theme) => ({
          color: theme.palette.text.primary,
          minWidth: 0,
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
        })}
      >
        {content || '—'}
      </Typography>
      <Chip size="sm" color="neutral" label={typeLabel} />
    </Stack>
  );
}

function TimestampCell({ value }: { value: string }) {
  return (
    <Stack spacing={0.25} sx={{ minWidth: 0 }}>
      <RelativeTimestamp timestamp={value} />
      <Typography
        variant="bodyXsRegular"
        sx={(theme) => ({ color: theme.palette.text.tertiary })}
      >
        {formatAbsoluteUtc(value)}
      </Typography>
    </Stack>
  );
}

export function TransactionSummary({ tx }: TransactionSummaryProps) {
  const memoType = tx.heavy?.memo_type;
  const memo = tx.heavy?.memo;
  const heavyUnavailable = tx.heavy == null;
  const failReason = tx.successful ? null : opFailReason(tx);

  return (
    <SectionCard
      title={
        <Stack direction="row" spacing={1.5} alignItems="center">
          <Typography variant="heading5SemiBold" component="h2">
            Summary
          </Typography>
          <StatusChip successful={tx.successful} />
        </Stack>
      }
    >
      {!tx.successful && (
        // Full-bleed status strip, not a floating alert box — the theme has
        // no MuiAlert style, so the strip borrows the error Chip's palette
        // (surface.error + text.error) to stay in the house style (0460 #8).
        <Box
          role="status"
          sx={(theme) => ({
            display: 'flex',
            alignItems: 'center',
            gap: 1,
            px: 2,
            py: 0.75,
            backgroundColor: theme.palette.surface.error,
            borderBottom: `1px solid ${theme.palette.stroke.error}`,
          })}
        >
          <ErrorOutlineIcon
            sx={(theme) => ({ fontSize: 16, color: theme.palette.text.error })}
          />
          <Typography
            variant="bodySmRegular"
            sx={(theme) => ({ color: theme.palette.text.error })}
          >
            Transaction failed — no operation was applied
            {/* The reason (task 0352). Two failure classes, one line each:
                an operation failed → that op's result code; the transaction
                never ran (validation) → the tx-level code, shown only when
                it says more than the sentence already did ("TxFailed" is
                the generic any-op-failed code, pure noise here). */}
            {failReason != null
              ? ` · ${failReason}`
              : heavyUnavailable
              ? // Without `heavy` we have neither per-op codes nor the tx-level
                // code, and a bare sentence would read as the validation-failure
                // class (the documented meaning of a null reason).
                ' · reason unavailable'
              : tx.heavy?.result_code != null &&
                tx.heavy.result_code.length > 0 &&
                tx.heavy.result_code !== 'TxFailed'
              ? ` · ${tx.heavy.result_code}`
              : ''}
          </Typography>
        </Box>
      )}
      <SummaryRow
        cells={[
          {
            label: 'Transaction hash',
            value: (
              <IdentifierWithCopy
                value={tx.hash}
                type="transaction"
                linked={false}
                truncate={false}
              />
            ),
          },
        ]}
      />
      <SummaryRow
        cells={[
          {
            label: 'Ledger',
            value: (
              <IdentifierDisplay
                value={String(tx.ledger_sequence)}
                type="ledger"
              />
            ),
          },
          {
            label: 'Timestamp',
            value: <TimestampCell value={tx.created_at} />,
          },
        ]}
      />
      <SummaryRow
        cells={[
          { label: 'Fee charged', value: <FeeCell stroops={tx.fee_charged} /> },
          {
            label: 'Memo',
            value: (
              <MemoCell
                memoType={memoType}
                memo={memo}
                unavailable={heavyUnavailable}
              />
            ),
          },
        ]}
      />
      <SummaryRow
        cells={[
          {
            label: 'Source account',
            value:
              tx.source_account != null ? (
                // Deliberately no SEP-2 name here. A transaction page carries
                // several accounts — source, operation destinations, signers
                // — and naming exactly one of them reads as "we know who the
                // sender is and not the recipient", which is false: we simply
                // did not ask. Naming all of them means a pair of
                // cross-origin requests per account, per page view, to hosts
                // those accounts named. The name lives one click away, on the
                // account's own page, which is the page whose subject the
                // account actually is (task 0443).
                <IdentifierWithCopy
                  value={tx.source_account}
                  type="account"
                  truncate={false}
                />
              ) : (
                <Dash />
              ),
          },
        ]}
      />
      {(tx.heavy?.fee_bump_source != null || tx.inner_tx_hash != null) && (
        <SummaryRow
          cells={[
            {
              label: 'Fee source',
              value:
                tx.heavy?.fee_bump_source != null ? (
                  <IdentifierWithCopy
                    value={tx.heavy.fee_bump_source}
                    type="account"
                    truncate={false}
                  />
                ) : heavyUnavailable ? (
                  // The row is reachable via the light `inner_tx_hash` alone,
                  // and a fee-bump envelope always HAS a fee source — a dash
                  // here would assert something impossible.
                  <UnavailableValue />
                ) : (
                  <Dash />
                ),
            },
            {
              label: 'Inner transaction',
              // Copy only, deliberately unlinked: inner hashes are not
              // indexed, so /transactions/{inner} would 404 (0359 plan #4).
              value:
                tx.inner_tx_hash != null ? (
                  <IdentifierWithCopy
                    value={tx.inner_tx_hash}
                    type="transaction"
                    linked={false}
                  />
                ) : (
                  <Dash />
                ),
            },
          ]}
        />
      )}
    </SectionCard>
  );
}
