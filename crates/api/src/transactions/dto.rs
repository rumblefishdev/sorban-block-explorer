//! Request and response DTOs for the transactions endpoints.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// `filter[...]` query parameters for `GET /v1/transactions`.
///
/// `limit` and `cursor` are read by a sibling `Pagination<TsIdCursor>`
/// extractor and documented via the handler's `#[utoipa::path(params(...))]`
/// attribute.
#[derive(Debug, Deserialize, IntoParams)]
pub struct ListParams {
    /// Filter by source account StrKey (G…).
    #[serde(rename = "filter[source_account]")]
    pub filter_source_account: Option<String>,
    /// Filter by contract StrKey (C…) — matches root op, nested call, or event emission.
    #[serde(rename = "filter[contract_id]")]
    pub filter_contract_id: Option<String>,
    /// Filter by operation type (e.g. `INVOKE_HOST_FUNCTION`).
    #[serde(rename = "filter[operation_type]")]
    pub filter_operation_type: Option<String>,
}

/// Opaque pagination payload for `GET /v1/transactions` (encoded via
/// [`common::cursor`](crate::common::cursor)).
///
/// The PG and CH read paths key their list scans on different columns, so
/// the cursor is a **datasource-tagged** enum rather than a shared superset:
///
/// - `Pg` — `(created_at, id)` keyset (`transactions.id` is a `BIGSERIAL`).
/// - `Ch` — `(ledger_sequence, id)` keyset with single-partition prune on
///   `intDiv(ledger_sequence, 500000)` (canonical SQL 02); `tiebreak` is the
///   `transactions.id` hash surrogate (the within-ledger tie-break).
///
/// The `src` tag makes the cursor self-describing. Per ADR 0008 the wire
/// format is opaque to clients, so the backend may change the encoding
/// freely; the flip side is that a cursor which decodes but carries a
/// legacy backend's intent MUST be rejected with `invalid_cursor` rather
/// than silently mis-paginating. `list_transactions` enforces that the
/// decoded variant is the current `Ch` keyset, and a legacy/untagged
/// cursor (pre-0243, no `src`) fails to decode at all — both surface as a
/// clean HTTP 400, exactly the "fail, don't silent-promote" contract ADR
/// 0008 prescribes. The tie-break is non-optional on the `Ch` variant, so a
/// CH keyset can never bind a NULL tuple element.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "src", rename_all = "snake_case")]
pub enum TxListCursor {
    Ch { ledger_sequence: i64, tiebreak: i64 },
}

/// Slim transaction row returned in the list endpoint.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TransactionListItem {
    /// Transaction hash (64-char lowercase hex).
    pub hash: String,
    pub ledger_sequence: i64,
    /// 1-based position of this transaction within its ledger.
    pub application_order: i16,
    /// `null` for Variant A `parse_error` transactions whose envelope
    /// could not be decoded (lore-0209). Always populated for ordinary
    /// (successful or failed-but-decoded) transactions.
    pub source_account: Option<String>,
    /// Fee charged, in raw stroops. Native (XLM) is always 7 decimals, so
    /// there is no `decimals` field — the frontend scales by 1e7.
    pub fee_charged: i64,
    /// Inner-transaction hash (64-char hex) for fee-bump envelopes, `null` otherwise.
    pub inner_tx_hash: Option<String>,
    pub successful: bool,
    pub operation_count: i16,
    /// `true` when the transaction touched at least one Soroban contract
    /// (root invocation, nested call, or event emission).
    pub has_soroban: bool,
    /// All distinct operation type names in the transaction
    /// (e.g. `["INVOKE_HOST_FUNCTION", "PAYMENT"]`).
    pub operation_types: Vec<String>,
    pub created_at: DateTime<Utc>,
}

// `memo_type` / `memo` are NOT exposed on the list item by design — list
// endpoints stay DB-only. Memo lives on the transaction detail endpoint
// (`GET /v1/transactions/{hash}`) inside the E3 `heavy` block, which
// already pays for the archive XDR fetch for the full transaction view.
// Adding memo here would require an archive fetch per ledger touched by
// the page, which is wasteful for the list use case and inconsistent
// with the DB-only contract advertised by canonical SQL 02.

/// DB-sourced light slice for the transaction detail endpoint.
///
/// Composed with `E3HeavyFields` via `merge_e3_response` (task 0150). All
/// XDR-sourced fields (memo, result_code, signatures, events, operation
/// details, envelope_xdr/result_xdr, operation_tree) live in `heavy`.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TransactionDetailLight {
    /// Transaction hash (64-char lowercase hex).
    pub hash: String,
    pub ledger_sequence: i64,
    /// 1-based position of this transaction within its ledger.
    pub application_order: i16,
    /// `null` for Variant A `parse_error` transactions whose envelope
    /// could not be decoded (lore-0209).
    pub source_account: Option<String>,
    /// The source account's on-chain `home_domain` (SEP-1 anchor domain),
    /// `null` when it sets none. The frontend asks that domain for the SEP-2
    /// federated address it claims for this account (task 0443, issue #363).
    pub source_account_home_domain: Option<String>,
    /// Fee charged, in raw stroops. Native (XLM) is always 7 decimals, so
    /// there is no `decimals` field — the frontend scales by 1e7.
    pub fee_charged: i64,
    /// Inner-transaction hash (64-char hex) for fee-bump envelopes, `null` otherwise.
    pub inner_tx_hash: Option<String>,
    pub successful: bool,
    pub operation_count: i16,
    pub has_soroban: bool,
    pub created_at: DateTime<Utc>,
    /// `true` when the XDR parser encountered an error for this transaction.
    pub parse_error: bool,
    pub operations: Vec<OperationItem>,
    /// Accounts touched by this transaction. Populated only when
    /// `heavy_fields_status = "unavailable"`; otherwise `[]` and consumers
    /// should rely on the heavy block.
    pub participants: Vec<String>,
    /// Soroban event appearance index rows. Same fallback semantics as
    /// `participants`. Full topics + data live in `heavy.contract_events`.
    pub soroban_events: Vec<EventAppearanceItem>,
    /// Soroban invocation appearance index rows. Same fallback semantics
    /// as `participants`. Full call hierarchy lives in `heavy.operation_tree`.
    pub soroban_invocations: Vec<InvocationAppearanceItem>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct EventAppearanceItem {
    pub contract_id: String,
    pub ledger_sequence: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct InvocationAppearanceItem {
    pub contract_id: String,
    /// Root caller G-StrKey. Per ADR 0034 nested-call hierarchy is XDR-only.
    pub caller_account: Option<String>,
    pub ledger_sequence: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct OperationItem {
    /// Global BIGSERIAL `operations_appearances.id`. Internal ordering
    /// artefact only; not a within-tx index. Use `application_order`
    /// for apply-order display and to join against
    /// `XdrOperationDto.application_order` from the heavy overlay.
    pub appearance_id: i64,
    /// Operation type tag in canonical SCREAMING_SNAKE_CASE
    /// (e.g. `"INVOKE_HOST_FUNCTION"`).
    pub type_name: String,
    /// Raw `OperationType` SMALLINT (ADR 0031).
    #[serde(rename = "type")]
    pub op_type: i16,
    pub source_account: Option<String>,
    pub destination_account: Option<String>,
    pub contract_id: Option<String>,
    /// Asset code (≤12 chars) for classic asset operations.
    pub asset_code: Option<String>,
    pub asset_issuer: Option<String>,
    /// Liquidity pools crossed by this operation, as SEP-23 strkeys
    /// (`L...`, 56 chars). Encoded from the DB hex form at the response
    /// boundary so cross-entity link targets match the
    /// `/v1/liquidity-pools/:id` route shape. Single-element for LP
    /// deposit/withdraw; the full crossed-pool list for path payments and
    /// offers that filled against a pool (task 0261/0268 — replaces the
    /// former nullable scalar `pool_id`).
    ///
    /// Empty `[]` means "no pool crossed" — authoritative on the ClickHouse
    /// read path, which extracts pool crossings from claim atoms across
    /// path-payment, offer, and LP deposit/withdraw ops.
    pub pool_ids: Vec<String>,
    /// 1-based per-tx apply position carrying on-chain operation order
    /// (task 0192). For folded appearance rows (multiple identical-identity
    /// envelope ops collapsed into one row, see task 0163) this is the
    /// MIN of the folded ops' indices — the position of the row's first
    /// occurrence in `tx.operations[]`. `None` for pre-task-0192 rows
    /// where the column was not yet populated; clients fall back to
    /// `appearance_id` order in that case.
    pub application_order: Option<i16>,
    pub ledger_sequence: i64,
    pub created_at: DateTime<Utc>,
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::cursor::{self, CursorError, Direction};
    use chrono::TimeZone;

    #[test]
    fn ch_cursor_round_trips() {
        // CH variant: ledger_sequence is the partition key + primary sort;
        // tiebreak is the transactions.id hash surrogate (the SQL `id`
        // column in the (ledger_sequence, id) keyset — may be negative,
        // cityhash64 lower bits as i64).
        let c = TxListCursor::Ch {
            ledger_sequence: 50_000,
            tiebreak: -123,
        };
        let encoded = cursor::encode(&c, Direction::Prev);
        let (dir, decoded): (Direction, TxListCursor) = cursor::decode(&encoded).unwrap();
        assert_eq!(dir, Direction::Prev);
        assert!(matches!(
            decoded,
            TxListCursor::Ch {
                ledger_sequence: 50_000,
                tiebreak: -123
            }
        ));
    }

    #[test]
    fn variant_carries_the_src_tag_on_the_wire() {
        // The `src` discriminant is what lets `list_transactions` reject a
        // stale PG cursor (ADR 0008 fail-clean): a decoded cursor without the
        // current `ch` tag is refused.
        assert_eq!(
            serde_json::to_value(TxListCursor::Ch {
                ledger_sequence: 1,
                tiebreak: 2
            })
            .unwrap()["src"],
            "ch"
        );
    }

    #[test]
    fn legacy_untagged_cursor_is_rejected() {
        // A pre-0243 `{ts, id}` cursor carries no `src` tag, so it MUST fail
        // to decode as the tagged enum — surfacing as `invalid_cursor` (400)
        // rather than silently promoting to a PG (or CH) walk. This is the
        // ADR 0008 "clean break / no silent-promotion" contract: a cursor
        // that lacks the current backend's intent fails, it does not
        // mis-paginate.
        #[derive(serde::Serialize)]
        struct Legacy {
            ts: DateTime<Utc>,
            id: i64,
        }
        let ts = Utc.with_ymd_and_hms(2026, 5, 29, 12, 0, 0).unwrap();
        let encoded = cursor::encode(&Legacy { ts, id: 7 }, Direction::Prev);
        let err = cursor::decode::<TxListCursor>(&encoded).unwrap_err();
        assert!(matches!(err, CursorError::InvalidPayload));
    }
}
