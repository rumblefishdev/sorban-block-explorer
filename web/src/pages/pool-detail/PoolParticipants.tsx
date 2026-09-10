import GroupIcon from '@mui/icons-material/GroupOutlined';
import { Typography } from '@mui/material';
import type { ParticipantItem } from '@rumblefish/api-types';
import {
  EmptyState,
  ExplorerTable,
  IdentifierDisplay,
  IdentifierWithCopy,
  PaginationControls,
  QueryErrorState,
  useCursorPagination,
  formatAmount,
  formatPercent,
  type ExplorerTableColumn,
} from '@rumblefish/soroban-block-explorer-ui';
import type { ReactNode } from 'react';

import { usePagedRows, usePoolParticipants } from '../../api/index.js';
import { CURSOR_PARAMS } from '../cursorParams.js';
import { SectionCard } from '../detail/SectionCard.js';

const columns: ExplorerTableColumn<ParticipantItem>[] = [
  {
    id: 'account',
    header: 'Account',
    width: 160,
    cell: (row) => <IdentifierWithCopy value={row.account} type="account" />,
  },
  {
    id: 'shares',
    header: 'Shares',
    align: 'right',
    width: 110,
    cell: (row) => (
      <Typography
        component="span"
        variant="bodySmMedium"
        sx={(theme) => ({ color: theme.palette.text.primary })}
      >
        {formatAmount(row.shares)}
      </Typography>
    ),
  },
  {
    id: 'share_percentage',
    header: 'Share %',
    align: 'right',
    width: 110,
    cell: (row) => (
      <Typography
        component="span"
        variant="bodySmMedium"
        sx={(theme) => ({ color: theme.palette.text.primary })}
      >
        {row.share_percentage != null
          ? formatPercent(Number(row.share_percentage))
          : '—'}
      </Typography>
    ),
  },
  {
    id: 'first_deposit_ledger',
    header: 'Since ledger',
    align: 'right',
    width: 120,
    cell: (row) => (
      <IdentifierDisplay
        value={String(row.first_deposit_ledger)}
        type="ledger"
      />
    ),
  },
];

interface PoolParticipantsProps {
  /** How many providers the pool is KNOWN to have, from the detail response.
   *  Lets the empty state tell "none" apart from "not listable". */
  knownParticipants?: number;
  poolId: string;
}

/**
 * "Pool participants" section of the LP detail page — a paginated list
 * of liquidity providers ordered by shares DESC. Fetched independently
 * of the rest of the page so failures stay scoped.
 */
export function PoolParticipants({
  poolId,
  knownParticipants,
}: PoolParticipantsProps) {
  // Namespaced cursor: LP detail mounts PoolParticipants + PoolActivity
  // simultaneously, so each section needs its own URL key. `resetKey`
  // drops the cursor when the user navigates to a different pool.
  const { cursor, goNext, goPrev } = useCursorPagination({
    cursorParam: CURSOR_PARAMS.POOL_PARTICIPANTS,
    resetKey: poolId,
  });

  const { data, isLoading, isPlaceholderData, isError, error, refetch } =
    usePoolParticipants(poolId, cursor);

  const { rows, canPrev, canNext, handlePrev, handleNext } = usePagedRows(
    data,
    goNext,
    goPrev
  );

  let body: ReactNode;
  if (isLoading || isPlaceholderData) {
    body = (
      <ExplorerTable
        columns={columns}
        rows={[]}
        rowKey={(row) => row.account}
        loading
        skeletonRows={20}
      />
    );
  } else if (isError) {
    body = <QueryErrorState error={error} onRetry={() => void refetch()} />;
  } else if (rows.length === 0) {
    // The KPI above counts providers from a per-asset aggregate, which this
    // list cannot page through: a Soroban pool's providers hold its share
    // TOKEN, and `balances` is ordered by holder, so listing the holders of
    // one asset is a full scan (measured: 113M rows, 4.22 GiB for a single
    // token — past the read-only profile). Saying "no participants" while the
    // strip says 337 would be the same contradiction from the other side, so
    // the section says which of the two it is.
    const countedButNotListed = (knownParticipants ?? 0) > 0;
    body = (
      <EmptyState
        icon={<GroupIcon />}
        title={
          countedButNotListed
            ? 'Participants not listed'
            : 'No participants yet'
        }
        description={
          countedButNotListed
            ? `This pool has ${knownParticipants} liquidity providers. Listing who they are is not indexed yet for this pool type.`
            : 'This pool currently has no active liquidity providers.'
        }
      />
    );
  } else {
    body = (
      <ExplorerTable
        columns={columns}
        rows={rows}
        rowKey={(row) => row.account}
      />
    );
  }

  return (
    <SectionCard title="Pool participants">
      {body}
      <PaginationControls
        caption="Latest results"
        canPrev={canPrev}
        canNext={canNext}
        onPrev={handlePrev}
        onNext={handleNext}
      />
    </SectionCard>
  );
}
