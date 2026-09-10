//! Wire-type contract for the display resolver, asserted against a REAL
//! ClickHouse.
//!
//! Two joins onto side tables mean two new decodes, and the failure mode is a
//! 500 on a live page rather than a compile error — the same class the sibling
//! statement's `toBool` and `CAST(… AS Array(Int64))` notes were paid for.
//!
//! Written so EMPTY tables still exercise every decode: the statements are
//! bounded by values that match nothing, so what is under test is that the
//! statement is accepted and its columns decode, independently of any fixture.
//!
//! ```text
//! docker compose up -d
//! CH_URL=http://localhost:8125 CH_USER=default CH_PASSWORD=clickhouse \
//!   cargo test -p api asset_identity
//! ```

use super::{AssetDisplay, IdentityKey, resolve_display_for};

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

/// One identity tuple, in the shape the side tables key on.
fn identity(
    id: i64,
    asset_type: i16,
    code: &str,
    issuer_id: i64,
    contract_id: i64,
) -> (i64, IdentityKey) {
    (id, (asset_type, code.to_string(), issuer_id, contract_id))
}

/// THE REAL STATEMENTS, over one identity of every family — native (empty code,
/// no issuer), classic credit, and a Soroban token (empty code, a contract).
/// The empty-code and zero-issuer values are the ones a naive bound would let
/// through as a wildcard, so they belong in the fixture rather than outside it.
#[tokio::test]
async fn the_display_statements_decode_every_family() {
    let Some(ch) = client() else {
        eprintln!("CH_URL unset — skipping display decode smoke");
        return;
    };
    let identities = [
        identity(-6_959_166_271_784_855_184, 0, "", 0, 0),
        identity(1, 1, "USDC", 42, 0),
        identity(2, 3, "", 0, 99),
    ];

    let got = resolve_display_for(&ch, &identities)
        .await
        .expect("both display statements must be accepted and decode");

    assert_eq!(
        got.len(),
        identities.len(),
        "every identity gets an entry, matched or not"
    );
}

/// An empty input must not build a statement with an empty `IN ()`, which is a
/// syntax error rather than an empty result.
#[tokio::test]
async fn an_empty_input_makes_no_statement() {
    let Some(ch) = client() else {
        eprintln!("CH_URL unset — skipping empty-input smoke");
        return;
    };
    let got = resolve_display_for(&ch, &[])
        .await
        .expect("empty input returns without querying");
    assert!(got.is_empty());
}

/// Pins the default: absent extras are `None`, never an empty string that
/// renders as a blank avatar or a dead link.
#[test]
fn the_default_display_is_two_absences() {
    let d = AssetDisplay::default();
    assert!(!d.sac_observed);
    assert!(d.icon_url.is_none());
    // An unobserved SAC yields no address even though the derivation itself
    // would happily produce one — the gate is the point.
    assert_eq!(
        super::sac_strkey(
            false,
            "USDC",
            "GA7QYNF7SOWQ3GLR2BGMZEHXAVIRZA4KVWLTJJFC7MGXUA74P7UJVSGZ",
            &[0u8; 32]
        ),
        None
    );
}
