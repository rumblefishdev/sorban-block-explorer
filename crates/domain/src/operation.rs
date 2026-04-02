//! Operation domain type matching the `operations` PostgreSQL table.

use serde::{Deserialize, Serialize};

/// Operation record as stored in PostgreSQL.
///
/// Partitioned by `transaction_id`. Composite PK: `(id, transaction_id)`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operation {
    /// Surrogate primary key (BIGSERIAL).
    pub id: i64,
    /// Parent transaction (FK to transactions.id, CASCADE).
    pub transaction_id: i64,
    /// Zero-based index within the transaction.
    pub application_order: i16,
    /// Source account (G... or M... address). Inherited from transaction if not overridden.
    pub source_account: String,
    /// Operation type string (e.g. "INVOKE_HOST_FUNCTION", "PAYMENT").
    pub op_type: String,
    /// Type-specific details stored as JSONB.
    pub details: serde_json::Value,
}
