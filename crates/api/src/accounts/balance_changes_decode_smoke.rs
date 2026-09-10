//! Wire-type contract, asserted against a REAL ClickHouse.
//!
//! A pure-Rust test cannot check that `sum()` over a `Nullable` column decodes
//! into `Option<String>`, that `toBool(...)` is what the driver needs to fill a
//! `bool`, or that an array literal's element type survives the round trip —
//! and getting any of them wrong is a 500 on a live page, not a compile error
//! (task 0324 took account-detail down this way, and the `Array(UInt64)` case
//! below reached a production account page).
//!
//! Runs against the repo's docker ClickHouse, which applies `init.sql` and so
//! has the tables these statements name:
//!
//! ```text
//! docker compose up -d
//! CH_URL=http://localhost:8123 CH_USER=default CH_PASSWORD=clickhouse \
//!   cargo test -p api decode_smoke
//! ```
//!
//! Read-only: every statement is a SELECT, and the ones that hit real tables
//! are written so an EMPTY table still exercises the decode.

use super::{DeltaChRow, TxKey, fetch_balance_changes};
use crate::common::asset_identity::{AssetIdentityChRow, resolve_asset_identities};
use std::collections::BTreeSet;

fn client() -> Option<clickhouse::Client> {
    let url = std::env::var("CH_URL").ok()?;
    let mut c = clickhouse::Client::default().with_url(url);
    if let Ok(u) = std::env::var("CH_USER") {
        c = c.with_user(u);
    }
    if let Ok(p) = std::env::var("CH_PASSWORD") {
        c = c.with_password(p);
    }
    Some(c)
}

/// THE REAL STATEMENT, with an id list whose values are ALL POSITIVE.
///
/// ClickHouse types an array literal from its values, so a bare
/// `[8106068169672383637]` is `Array(UInt64)` and the `id` column comes
/// back `UInt64`, which does not decode into `i64`. That is a 500 on
/// exactly the accounts whose assets all hash positive and on no others —
/// it reached production account `GBO56XB4…` (its only asset is `XTAR`)
/// while every test passed, because native XLM's surrogate is NEGATIVE and
/// every fixture happened to include it.
///
/// Asserted through the real builder rather than a copy of its SQL: a
/// hand-written literal would have to repeat the `CAST`, which is the very
/// thing under test.
#[tokio::test]
async fn the_identity_statement_decodes_an_all_positive_id_list() {
    let Some(ch) = client() else {
        eprintln!("CH_URL unset — skipping identity decode smoke");
        return;
    };
    let ids: BTreeSet<i64> = [8_106_068_169_672_383_637, 42, i64::MAX]
        .into_iter()
        .collect();
    let got = resolve_asset_identities(&ch, &ids)
        .await
        .expect("an all-positive id list must decode as Int64");
    assert_eq!(
        got.len(),
        ids.len(),
        "every id must come back, matched or not"
    );
    assert!(got.contains_key(&i64::MAX));
}

/// The mirror: all-negative, which is what an XLM-bearing page produces.
/// `Array(Int64)` either way — the `CAST` must not have made the common
/// case worse.
#[tokio::test]
async fn the_identity_statement_decodes_an_all_negative_id_list() {
    let Some(ch) = client() else {
        eprintln!("CH_URL unset — skipping negative identity decode smoke");
        return;
    };
    let ids: BTreeSet<i64> = [-6_959_166_271_784_855_184, -1, i64::MIN]
        .into_iter()
        .collect();
    let got = resolve_asset_identities(&ch, &ids)
        .await
        .expect("an all-negative id list must decode as Int64");
    assert_eq!(got.len(), ids.len());
}

/// THE REAL delta statement, against the real table. The account and keys
/// match nothing, so this asserts the statement is ACCEPTED — its types,
/// its `HAVING` over a `Nullable`, its partition prune — independently of
/// any fixture. An empty page is also the shape a live account with no
/// transfers produces, so it is worth pinning on its own.
#[tokio::test]
async fn the_delta_statement_runs_and_an_empty_page_is_empty() {
    let Some(ch) = client() else {
        eprintln!("CH_URL unset — skipping delta statement smoke");
        return;
    };
    let keys = [
        TxKey {
            ledger_sequence: 64_318_000,
            application_order: 1,
            transaction_id: 1,
        },
        TxKey {
            ledger_sequence: 1,
            application_order: 0,
            transaction_id: 2,
        },
    ];
    let got = fetch_balance_changes(&ch, i64::MIN, &keys)
        .await
        .expect("the delta statement must be accepted by the server");
    assert!(got.is_empty(), "no account matches i64::MIN");
}

#[tokio::test]
async fn delta_row_decodes_a_nullable_sum() {
    let Some(ch) = client() else {
        eprintln!("CH_URL unset — skipping balance-change decode smoke");
        return;
    };
    // Both arms in one result: a fungible asset whose sum is a number, and
    // a non-fungible one where every row carried NULL and the sum is NULL.
    // `sum()` over `Nullable(Int128)` is `Nullable`, so `toString` of it is
    // `Nullable(String)` — decoding that into a plain `String` 500s.
    let rows = ch
        .query(
            "SELECT ledger_sequence, application_order, asset_id, \
                    toString(sum(signed)) AS delta, sum(nft) AS nft_delta, \
                    nft_owner \
             FROM ( \
                 SELECT toInt64(64318000) AS ledger_sequence, \
                        toInt16(1)        AS application_order, \
                        toInt64(7)        AS asset_id, \
                        CAST(60970653780 AS Nullable(Int128)) AS signed, \
                        toInt16(0)        AS nft, \
                        CAST(NULL AS Nullable(Int64)) AS nft_owner \
                 UNION ALL \
                 SELECT toInt64(64318002), toInt16(3), toInt64(9), \
                        CAST(NULL AS Nullable(Int128)), toInt16(-1), \
                        CAST(42 AS Nullable(Int64)) \
             ) \
             GROUP BY ledger_sequence, application_order, asset_id, nft_owner \
             ORDER BY ledger_sequence",
        )
        .fetch_all::<DeltaChRow>()
        .await
        .expect("delta row must decode");

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].delta.as_deref(), Some("60970653780"));
    assert_eq!(rows[0].nft_delta, 0);
    assert_eq!(rows[1].delta, None, "an all-NULL sum must stay NULL, not 0");
    assert_eq!(rows[1].nft_delta, -1);
    assert_eq!(
        rows[1].nft_owner,
        Some(42),
        "the new owner keys the piece lookup"
    );
}

/// An amount past `i64` — which is why the column is `Int128` and why it
/// crosses the wire as a STRING. A token with 18 decimals reaches this
/// range on an ordinary balance, so it is not a theoretical case.
#[tokio::test]
async fn an_amount_beyond_i64_survives_as_a_string() {
    let Some(ch) = client() else {
        eprintln!("CH_URL unset — skipping Int128 magnitude smoke");
        return;
    };
    let rows = ch
        .query(
            "SELECT toInt64(1) AS ledger_sequence, toInt16(1) AS application_order, \
                    toInt64(1) AS asset_id, \
                    toString(sum(signed)) AS delta, sum(nft) AS nft_delta, \
                    CAST(NULL AS Nullable(Int64)) AS nft_owner \
             FROM (SELECT CAST('-170141183460469231731687303715884105728' \
                           AS Nullable(Int128)) AS signed, toInt16(0) AS nft)",
        )
        .fetch_all::<DeltaChRow>()
        .await
        .expect("an Int128 at its bound must decode");
    assert_eq!(
        rows[0].delta.as_deref(),
        Some("-170141183460469231731687303715884105728")
    );
    assert_eq!(
        rows[0].delta.as_deref().unwrap().parse::<i128>().unwrap(),
        i128::MIN
    );
}

#[tokio::test]
async fn asset_identity_row_decodes_bool_and_lowcardinality_nullables() {
    let Some(ch) = client() else {
        eprintln!("CH_URL unset — skipping asset identity decode smoke");
        return;
    };
    // `toBool(...)`, not the bare comparison: `a.id != 0` is `UInt8` and the
    // driver fills a Rust `bool` from CH `Bool`. `asset_code` comes back as
    // `LowCardinality(Nullable(String))` from the real column.
    let rows = ch
        .query(
            "SELECT toInt64(7) AS id, \
                    toBool(1)  AS known, \
                    toInt16(1) AS asset_type, \
                    nullIf(CAST('USDC' AS LowCardinality(String)), '') AS asset_code, \
                    toInt64(42) AS issuer_id, \
                    toInt64(0)  AS contract_id, \
                    nullIf('', '') AS contract_strkey, \
                    nullIf('', '') AS symbol, \
                    coalesce(CAST(NULL AS Nullable(UInt32)), 7) AS decimals, \
                    toBool(true) AS decimals_known",
        )
        .fetch_all::<AssetIdentityChRow>()
        .await
        .expect("asset identity row must decode");

    assert_eq!(rows.len(), 1);
    assert!(rows[0].known);
    assert_eq!(rows[0].asset_code.as_deref(), Some("USDC"));
    assert_eq!(rows[0].contract_strkey, None);
    assert_eq!(rows[0].decimals, 7);
    // The flag rides the same row; a classic asset's 7 is protocol, not a guess.
    assert!(rows[0].decimals_known);
}
