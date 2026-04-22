//! Pure merge functions that fold DB-sourced light fields together with
//! XDR-sourced heavy fields into the final endpoint response DTOs.
//!
//! These functions are generic over the caller's DB-side "light" shape so the
//! same helper works whether the light type comes from `domain::*` or from a
//! handler-local DTO. No I/O, no allocation beyond the result struct.

use super::dto::{
    E3HeavyFields, E3Response, E14EventResponse, E14HeavyEventFields, HeavyFieldsStatus,
};

/// Merge the DB light view of a transaction with the XDR heavy fields.
///
/// `heavy = Some(_)` → `heavy_fields_status: "ok"`.
/// `heavy = None` (upstream fetch failed) → `heavy_fields_status: "unavailable"`
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

/// Merge a single DB light event row with its XDR heavy payload.
///
/// When `heavy = None`, topics and data are absent (DB-only fallback).
pub fn merge_e14_event_response<E>(
    light: E,
    heavy: Option<E14HeavyEventFields>,
) -> E14EventResponse<E> {
    let (topics, data, status) = match heavy {
        Some(h) => (Some(h.topics), Some(h.data), HeavyFieldsStatus::Ok),
        None => (None, None, HeavyFieldsStatus::Unavailable),
    };
    E14EventResponse {
        light,
        topics,
        data,
        heavy_fields_status: status,
    }
}

/// Correlate a slice of DB light events against a slice of XDR heavy events
/// emitted by the same contract within one ledger, producing merged rows in
/// the order of the DB input.
///
/// Matching is done on `(transaction_hash, event_index)` — the XDR-side event
/// uses hex tx hash, the DB side uses the same. Callers supply an extractor
/// closure to pull `(tx_hash_hex, event_index)` from their light type so this
/// function stays generic.
///
/// If no matching XDR heavy event is found, the resulting merged row has
/// `heavy_fields_status: "unavailable"` — equivalent to a missing upstream.
pub fn merge_e14_events<E, F>(
    light: Vec<E>,
    heavy: Vec<E14HeavyEventFields>,
    key_of_light: F,
) -> Vec<E14EventResponse<E>>
where
    F: Fn(&E) -> (String, i16),
{
    // Small N (page size ≤100 on E14); linear lookup beats hashing overhead.
    light
        .into_iter()
        .map(|l| {
            let (tx_hash, event_index) = key_of_light(&l);
            let matched = heavy
                .iter()
                .find(|h| h.transaction_hash == tx_hash && h.event_index == event_index);
            merge_e14_event_response(l, matched.cloned())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, serde::Serialize)]
    struct TxLight {
        hash: String,
        successful: bool,
    }

    #[derive(Debug, Clone, PartialEq, serde::Serialize)]
    struct EventLight {
        transaction_hash: String,
        event_index: i16,
        topic0: Option<String>,
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

    #[test]
    fn merge_e14_matches_by_hash_and_index() {
        let light = vec![
            EventLight {
                transaction_hash: "aa".into(),
                event_index: 0,
                topic0: Some("transfer".into()),
            },
            EventLight {
                transaction_hash: "bb".into(),
                event_index: 1,
                topic0: None,
            },
        ];
        let heavy = vec![E14HeavyEventFields {
            event_index: 1,
            transaction_hash: "bb".into(),
            topics: vec![serde_json::json!("t0"), serde_json::json!("t1")],
            data: serde_json::json!({"amount": "100"}),
        }];
        let merged = merge_e14_events(light, heavy, |e| {
            (e.transaction_hash.clone(), e.event_index)
        });

        assert_eq!(merged.len(), 2);
        assert_eq!(
            merged[0].heavy_fields_status,
            HeavyFieldsStatus::Unavailable
        );
        assert!(merged[0].topics.is_none());
        assert_eq!(merged[1].heavy_fields_status, HeavyFieldsStatus::Ok);
        assert_eq!(merged[1].topics.as_ref().unwrap().len(), 2);
    }
}
