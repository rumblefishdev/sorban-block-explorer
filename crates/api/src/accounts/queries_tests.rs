//! Predicates of the accounts read that a rename or a one-word edit would
//! silently break, asserted against the SQL itself — no ClickHouse needed.

use super::{BALANCES_SQL, asset_type_name};

/// The account-detail read must select on the LIFECYCLE column, never on
/// the amount. `amount != 0` cannot tell "holds nothing" from "the
/// trustline is gone" — it was the defect behind issue #377, and it is a
/// one-word regression away, so the predicate is pinned here rather than
/// left to review. Asserted against the SQL itself, no ClickHouse needed.
#[test]
fn balances_are_selected_by_lifecycle_not_by_amount() {
    assert!(
        BALANCES_SQL.contains("b.closed_at_ledger = 0"),
        "the balances read must filter on the lifecycle column"
    );
    assert!(
        !BALANCES_SQL.contains("b.amount != 0"),
        "`amount != 0` hides every zero-balance trustline the account holds"
    );
    // A zero-amount row that is still open has to survive the predicate,
    // which is only true if `amount` is absent from the WHERE clause
    // entirely — a combined `amount != 0 OR ...` would pass the check above.
    let where_clause = BALANCES_SQL
        .split("WHERE")
        .nth(1)
        .expect("the read has a WHERE clause");
    assert!(
        !where_clause.contains("amount"),
        "no amount predicate belongs in this WHERE clause: {where_clause}"
    );
}

/// The old version of this test pinned the XDR legend onto family values
/// and thereby froze bug 0496 in place: it asserted 3 = `pool_share`, so
/// every Soroban holding rendered as a liquidity-pool share and the test
/// was green. A parity test is only as good as the enum it picks.
#[test]
fn asset_type_name_speaks_the_family_vocabulary() {
    assert_eq!(asset_type_name(0).as_deref(), Some("native"));
    assert_eq!(asset_type_name(1).as_deref(), Some("classic_credit"));
    assert_eq!(
        asset_type_name(3).as_deref(),
        Some("soroban"),
        "3 is AssetFamily::Soroban here, never the XDR pool_share"
    );
    assert_eq!(asset_type_name(2), None, "2 (sac) is retired — ADR 0051");
    assert_eq!(asset_type_name(99), None);
}
