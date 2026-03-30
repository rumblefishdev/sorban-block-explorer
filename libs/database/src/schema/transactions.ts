import {
  bigint,
  bigserial,
  boolean,
  index,
  jsonb,
  pgTable,
  text,
  timestamp,
  varchar,
} from 'drizzle-orm/pg-core';
import { relations } from 'drizzle-orm';
import { ledgers } from './ledgers.js';

export const transactions = pgTable(
  'transactions',
  {
    id: bigserial('id', { mode: 'bigint' }).primaryKey(),
    hash: varchar('hash', { length: 64 }).unique().notNull(),
    ledgerSequence: bigint('ledger_sequence', { mode: 'bigint' })
      .references(() => ledgers.sequence)
      .notNull(),
    sourceAccount: varchar('source_account', { length: 56 }).notNull(),
    feeCharged: bigint('fee_charged', { mode: 'bigint' }).notNull(),
    successful: boolean('successful').notNull(),
    resultCode: varchar('result_code', { length: 50 }),
    envelopeXdr: text('envelope_xdr').notNull(),
    resultXdr: text('result_xdr').notNull(),
    resultMetaXdr: text('result_meta_xdr'),
    memoType: varchar('memo_type', { length: 20 }),
    memo: text('memo'),
    createdAt: timestamp('created_at', { withTimezone: true }).notNull(),
    parseError: boolean('parse_error').default(false),
    operationTree: jsonb('operation_tree'),
  },
  (table) => [
    index('idx_source').on(table.sourceAccount, table.createdAt.desc()),
    index('idx_ledger').on(table.ledgerSequence),
  ]
);

export const transactionsRelations = relations(transactions, ({ one }) => ({
  ledger: one(ledgers, {
    fields: [transactions.ledgerSequence],
    references: [ledgers.sequence],
  }),
}));
