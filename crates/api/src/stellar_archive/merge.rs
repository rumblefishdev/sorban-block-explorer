//! Pure composition helpers that combine DB-sourced light slices with
//! XDR-sourced fields into the final endpoint response DTOs.
//!
//! ADR 0033: E14 no longer has a DB-side event row to merge against — its
//! response is assembled directly in the handler from appearance rows plus
//! parser output, so no E14 helper lives here. E3 still merges a DB tx-light
//! slice with an XDR heavy struct; that helper stays generic over the
//! caller's light type.

use super::dto::{E3HeavyFields, E3Response, HeavyFieldsStatus};

/// Merge the DB light view of a transaction with the XDR heavy fields.
///
/// `heavy = Some(_)` → `heavy_fields_status: Ok`.
/// `heavy = None` (upstream fetch failed) → `heavy_fields_status: Unavailable`
/// and the response still contains the DB light view.
pub fn merge_e3_response<T>(light: T, heavy: Option<E3HeavyFields>) -> E3Response<T> {
    let heavy_fields_status = if heavy.is_some() {
        HeavyFieldsStatus::Ok
    } else {
        HeavyFieldsStatus::Unavailable
    };
    E3Response {
        light,
        heavy,
        heavy_fields_status,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, serde::Serialize)]
    struct TxLight {
        hash: String,
        successful: bool,
    }

    fn sample_heavy() -> E3HeavyFields {
        E3HeavyFields {
            memo_type: Some("text".into()),
            memo: Some("hi".into()),
            signatures: Vec::new(),
            fee_bump_source: None,
            envelope_xdr: Some("AAAA".into()),
            result_xdr: Some("AAAB".into()),
            result_meta_xdr: None,
            diagnostic_events: Vec::new(),
            contract_events: Vec::new(),
            invocations: Vec::new(),
            operations: Vec::new(),
        }
    }

    #[test]
    fn merge_e3_ok_when_heavy_some() {
        let light = TxLight {
            hash: "abc".into(),
            successful: true,
        };
        let merged = merge_e3_response(light, Some(sample_heavy()));
        assert_eq!(merged.heavy_fields_status, HeavyFieldsStatus::Ok);
        assert!(merged.heavy.is_some());
    }

    #[test]
    fn merge_e3_unavailable_when_heavy_none() {
        let light = TxLight {
            hash: "abc".into(),
            successful: true,
        };
        let merged = merge_e3_response(light, None);
        assert_eq!(merged.heavy_fields_status, HeavyFieldsStatus::Unavailable);
        assert!(merged.heavy.is_none());
    }
}
