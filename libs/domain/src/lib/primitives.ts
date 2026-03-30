// --- Shared type primitives ---

export type JsonValue =
  | string
  | number
  | boolean
  | null
  | readonly JsonValue[]
  | { readonly [key: string]: JsonValue };

/** Decoded Soroban ScVal. Placeholder until task 0013 provides a full ScVal type. */
export type ScVal = JsonValue;

/** String representation of a PostgreSQL BIGINT/BIGSERIAL value. */
export type BigIntString = string;

/** String representation of a PostgreSQL NUMERIC / DECIMAL value. */
export type NumericString = string;
