import type { BigIntString, JsonValue, ScVal } from './primitives.js';

// --- Operation domain types ---

export type OperationType =
  | 'CREATE_ACCOUNT'
  | 'PAYMENT'
  | 'PATH_PAYMENT_STRICT_RECEIVE'
  | 'PATH_PAYMENT_STRICT_SEND'
  | 'MANAGE_SELL_OFFER'
  | 'MANAGE_BUY_OFFER'
  | 'CREATE_PASSIVE_SELL_OFFER'
  | 'SET_OPTIONS'
  | 'CHANGE_TRUST'
  | 'ALLOW_TRUST'
  | 'ACCOUNT_MERGE'
  | 'INFLATION'
  | 'MANAGE_DATA'
  | 'BUMP_SEQUENCE'
  | 'CREATE_CLAIMABLE_BALANCE'
  | 'CLAIM_CLAIMABLE_BALANCE'
  | 'BEGIN_SPONSORING_FUTURE_RESERVES'
  | 'END_SPONSORING_FUTURE_RESERVES'
  | 'REVOKE_SPONSORSHIP'
  | 'CLAWBACK'
  | 'CLAWBACK_CLAIMABLE_BALANCE'
  | 'SET_TRUST_LINE_FLAGS'
  | 'LIQUIDITY_POOL_DEPOSIT'
  | 'LIQUIDITY_POOL_WITHDRAW'
  | 'INVOKE_HOST_FUNCTION'
  | 'EXTEND_FOOTPRINT_TTL'
  | 'RESTORE_FOOTPRINT'
  | (string & {});

export interface InvokeHostFunctionDetails {
  contractId: string;
  functionName: string;
  functionArgs: readonly ScVal[];
  returnValue: ScVal | null;
}

export interface Operation {
  id: BigIntString;
  transactionId: BigIntString;
  type: OperationType;
  details: Readonly<Record<string, JsonValue>>;
}
