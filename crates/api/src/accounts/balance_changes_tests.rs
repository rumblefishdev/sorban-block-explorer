//! The load-bearing clauses of the balance-change statement, asserted against
//! the built SQL — each is one word away from a regression that review does not
//! catch, and none of them needs a ClickHouse to pin. The wire-type contract
//! that DOES need a server lives in `decode_smoke` beside them.

use super::*;

#[test]
fn a_transfer_to_self_nets_to_zero() {
    // `to − from`, never a first-match branch: with `from_id = to_id = A` a
    // `multiIf` that tests `to_id` first returns `+amount` and the page shows
    // an account paying itself. Both legs must fire and cancel.
    let sql = balance_change_delta_sql(42, "(1,1)", "0");
    assert!(
        sql.contains("if(to_id = 42, amount, 0) - if(from_id = 42, amount, 0)"),
        "signed amount must subtract the outgoing leg, not branch on it: {sql}"
    );
}

#[test]
fn the_read_dedups_and_prunes() {
    let sql = balance_change_delta_sql(42, "(1,1)", "0");
    // `asset_transfers` is a version-less ReplacingMergeTree and production
    // tables here carry unmerged duplicates permanently (0420) — without the
    // inner group-by over the FULL sort key a duplicate doubles the amount.
    assert!(
        sql.contains(
            "GROUP BY ledger_sequence, application_order, op_index, event_pos_in_op, asset_id"
        ),
        "inner dedup over the full sort key is missing: {sql}"
    );
    // PARTITION BY intDiv(ledger_sequence, 500000) — the key filter alone
    // does not prune.
    assert!(
        sql.contains("intDiv(ledger_sequence, 500000) IN"),
        "partition prune is missing: {sql}"
    );
}

#[test]
fn a_non_fungible_movement_never_sums_as_zero() {
    let sql = balance_change_delta_sql(42, "(1,1)", "0");
    // A `{token_id}` movement has no amount by nature. It is counted, signed,
    // in its own column; folding it into the amount sum would print a real
    // "balance change: 0" for a transaction that changed an owner.
    assert!(
        sql.contains("if(amount IS NULL,"),
        "non-fungible movements must be counted separately: {sql}"
    );
    assert!(
        sql.contains("sum(nft) AS nft_delta"),
        "non-fungible count must survive the outer aggregate: {sql}"
    );
    // NULL and 0 are different answers, so the HAVING must not conflate them.
    assert!(
        sql.contains("HAVING (delta IS NOT NULL AND delta != '0') OR nft_delta != 0"),
        "HAVING must keep non-fungible rows and drop only measured zeros: {sql}"
    );
}

#[test]
fn assets_come_back_in_the_order_they_moved() {
    // Assets have different decimals and different prices, and we hold no
    // prices at all — so any ranking by amount compares quantities that are
    // not comparable. The order the movements HAPPENED in is a fact the chain
    // supplies, and it is the one the cell shows (`[0]` plus `+N`), so it has
    // to come from the statement and must not be re-sorted afterwards.
    let sql = balance_change_delta_sql(42, "(1,1)", "0");
    assert!(
        sql.contains(
            "ORDER BY ledger_sequence, application_order, min((op_index, event_pos_in_op))"
        ),
        "assets must be ordered by their first movement in the transaction: {sql}"
    );
}

#[test]
fn the_floor_is_the_ledger_the_indexer_started_writing() {
    // Below it the table is EMPTY, which is indistinguishable from "nothing
    // moved" — the API must report absence, never a zero. Pinned so the value
    // moves only deliberately, with the backfill that earns it.
    assert_eq!(VALUE_FLOW_FLOOR_LEDGER, 64_317_019);
}
