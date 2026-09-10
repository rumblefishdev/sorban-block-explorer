//! Guards on the shared identity statements — no ClickHouse needed.
//! Behavioural coverage of the same statements lives in
//! `asset_identity_decode_smoke.rs`, which does need one.

use super::*;

/// The SAC read must not filter an asset out for having an EMPTY `asset_code`
/// (task 0470).
///
/// An empty code is native XLM's real, stored identity — not a missing value —
/// and native has a deployed SAC. An `asset_code != ''` guard was added
/// deliberately in `a19ac8f6` to match Postgres, which returned NULL there;
/// Postgres is retired and `/v1/assets/native` publishes that same SAC, so the
/// guard left one asset describing itself two ways depending on the endpoint.
///
/// Pinned on the module source because the statements are inline string
/// literals — there is no builder to call. That is the honest limit of this
/// guard: it catches the exact regression (a re-added `!= ''` on an asset code)
/// and nothing subtler. A behavioural test needs the statements extracted
/// first, which is recorded as an acceptance criterion on 0470.
///
/// It used to scan `liquidity_pools/queries.rs` for `asset_a_code != ''`. Task
/// 0374 moved every leg identity read into THIS module and deleted those
/// columns, at which point the guard was scanning a file that could no longer
/// contain its subject — a test that cannot fail, reading as coverage.
#[test]
fn no_code_guard_can_exclude_the_native_asset_from_its_sac() {
    // Only the production half: this file quotes the guard it looks for.
    let src = include_str!("asset_identity.rs");
    let production = src.split("#[cfg(test)]").next().unwrap_or(src);
    let guards = production
        .lines()
        .filter(|l| !l.trim_start().starts_with("//") && !l.trim_start().starts_with("///"))
        .filter(|l| l.contains("asset_code != ''") || l.contains("asset_code > ''"))
        .count();
    assert_eq!(
        guards, 0,
        "an asset-code guard is back: it silently drops native XLM's SAC, \
         which /v1/assets/native still reports"
    );
}

/// The display rule has ONE producer, and the native arm is the reason.
#[test]
fn the_shown_code_names_native_by_its_type() {
    let sql = shown_code_sql("a.");
    assert!(sql.contains("a.asset_type = 0"), "{sql}");
    assert!(sql.contains("'XLM'"), "{sql}");
    // Unaliased form, for a query that does not alias `assets`.
    assert!(shown_code_sql("").contains("if(asset_type = 0"));
}

/// An identity the dimension does not know carries no display extras, and must
/// not reach the statement at all: `known == false` is the NFT-collection case
/// and the unclassified-contract case, both real and both legitimately blank.
/// Its empty code and zero issuer would also widen the bounds if they did.
#[test]
fn unknown_identities_get_no_display_key() {
    let rows = vec![
        AssetIdentityChRow {
            id: 1,
            known: true,
            asset_type: 1,
            asset_code: Some("USDC".into()),
            issuer_id: 42,
            contract_id: 0,
            contract_strkey: None,
            symbol: None,
            decimals: 7,
        },
        AssetIdentityChRow {
            id: 7,
            known: false,
            asset_type: 3,
            asset_code: None,
            issuer_id: 0,
            contract_id: 7,
            contract_strkey: None,
            symbol: None,
            decimals: 7,
        },
    ];
    let keys = display_keys(&rows);
    assert_eq!(keys.len(), 1, "the unknown row must not produce a key");
    assert_eq!(keys[0], (1, (1i16, "USDC".to_string(), 42, 0)));
}

/// Native's code is EMPTY on the ledger, and the tuple carries it that way —
/// the side tables key on the stored value, not on the displayed `XLM`.
#[test]
fn a_native_key_carries_the_empty_stored_code() {
    let rows = vec![AssetIdentityChRow {
        id: -1,
        known: true,
        asset_type: 0,
        asset_code: None,
        issuer_id: 0,
        contract_id: 0,
        contract_strkey: None,
        symbol: None,
        decimals: 7,
    }];
    assert_eq!(display_keys(&rows)[0].1, (0i16, String::new(), 0, 0));
}
