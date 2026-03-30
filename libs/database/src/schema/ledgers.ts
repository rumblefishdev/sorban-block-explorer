import {
  bigint,
  index,
  pgTable,
  varchar,
  integer,
  timestamp,
} from 'drizzle-orm/pg-core';

export const ledgers = pgTable(
  'ledgers',
  {
    sequence: bigint('sequence', { mode: 'bigint' }).primaryKey(),
    hash: varchar('hash', { length: 64 }).unique().notNull(),
    closedAt: timestamp('closed_at', { withTimezone: true }).notNull(),
    protocolVersion: integer('protocol_version').notNull(),
    transactionCount: integer('transaction_count').notNull(),
    baseFee: bigint('base_fee', { mode: 'bigint' }).notNull(),
  },
  (table) => [index('idx_closed_at').on(table.closedAt.desc())]
);
