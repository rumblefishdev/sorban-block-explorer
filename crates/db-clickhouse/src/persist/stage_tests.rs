//! Sibling tests for `stage.rs`.
//!
//! `stage.rs` still carries several `#[cfg(test)] mod` blocks inline, which the
//! repo rule forbids and the 0525 ratchet is meant to unwind. This file is the
//! destination they move to; new tests start here rather than adding to the
//! inline stock.

use std::collections::HashMap;

use super::*;

/// Native XLM's Stellar Asset Contract on pubnet — the single most common leg
/// token across soroban pools (211 pools at the 2026-09-08 measurement).
const XLM_SAC: &str = "CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA";
/// A Soroban-native token: no classic asset behind it, so no SAC mapping and
/// its contract surrogate legitimately IS its asset id.
const SOROBAN_TOKEN: &str = "CDXODVQDUB6DFAQB77UBKJO6KDK7A3KIB5SJKMJF27R6NN6OB742NPSX";
const POOL: &str = "CCH6A2JCUFDNPPV2XHG5SPIOGQREV7QJZ62CIZJXQQR7E3NI3YZXBJIF";
const DEPLOYER: &str = "CARVO4GFHVVHSNJQUJGWINRNTL3Z6LRR3YIL54LGQSDWO4LHXCY5IMCZ";

/// The map the balance path already builds, with the one entry these tests need.
fn sac_map() -> HashMap<i64, i64> {
    HashMap::from([(ids::contract_id(XLM_SAC), ids::NATIVE_ASSET_ID)])
}

/// Asserts the invariant all three families share: a SAC leg carries the
/// WRAPPED asset's id, a Soroban-native leg keeps its own contract surrogate,
/// and the deployment is never re-keyed (a factory is not an asset).
fn assert_legs_rekeyed(row: &LiquidityPoolRow) {
    assert_eq!(
        row.legs[0],
        ids::NATIVE_ASSET_ID,
        "a SAC leg must key onto the classic/native asset it wraps (ADR 0051)",
    );
    assert_ne!(
        row.legs[0],
        ids::contract_id(XLM_SAC),
        "a SAC leg keyed on its own contract surrogate has no `assets` row — \
         this is the orphaning defect the re-keying exists to prevent",
    );
    assert_eq!(
        row.legs[1],
        ids::contract_id(SOROBAN_TOKEN),
        "a Soroban-native leg IS its contract, and must pass through unchanged",
    );
    assert_eq!(
        row.deployment_id,
        ids::contract_id(DEPLOYER),
        "the registering factory/router is not an asset and is never re-keyed",
    );
}

/// Router family (Aquarius). The regression this guards: the leg map was
/// `ids::contract_id` applied straight to every token.
#[test]
fn router_registration_rekeys_a_sac_leg() {
    let event = xdr_parser::pool_router::AddPoolEvent {
        pool: POOL.to_string(),
        pool_type: "constant".to_string(),
        tokens: vec![XLM_SAC.to_string(), SOROBAN_TOKEN.to_string()],
        init_args: vec!["30".to_string()],
    };

    let row = pool_registry_row(&event, DEPLOYER, 50_875_676, &sac_map())
        .expect("a well-formed add_pool registration builds a row");

    assert_legs_rekeyed(&row);
}

/// Pair-factory family (Soroswap). Same defect, separate builder — the three
/// arms are why this is tested per family rather than once on the helper.
#[test]
fn pair_factory_registration_rekeys_a_sac_leg() {
    let reg = xdr_parser::pool_pair_factory::PairRegistration {
        factory: DEPLOYER.to_string(),
        event: xdr_parser::pool_pair_factory::NewPairEvent {
            pair: POOL.to_string(),
            token_0: XLM_SAC.to_string(),
            token_1: SOROBAN_TOKEN.to_string(),
            new_pairs_length: 1,
        },
    };

    let row = factory_pair_registry_row(&reg, 50_875_676, &sac_map())
        .expect("a well-formed new_pair registration builds a row");

    assert_legs_rekeyed(&row);
}

/// Config-factory family (Phoenix), whose legs come from the pool's own CONFIG
/// rather than from the event.
#[test]
fn config_factory_registration_rekeys_a_sac_leg() {
    let reg = xdr_parser::pool_config_factory::ConfigPoolRegistration {
        factory: DEPLOYER.to_string(),
        pool: POOL.to_string(),
        ledger_sequence: 50_875_676,
    };
    let config = xdr_parser::pool_config_factory::PoolConfig {
        token_a: XLM_SAC.to_string(),
        token_b: SOROBAN_TOKEN.to_string(),
        share_token: SOROBAN_TOKEN.to_string(),
        pool_type: 0,
        total_fee_bps: 30,
    };

    let row = config_pool_registry_row(&reg, &config, 50_875_676, &sac_map())
        .expect("a well-formed CONFIG builds a row");

    assert_legs_rekeyed(&row);
}

/// An EMPTY map is the legacy-caller case, and it must not invent a mapping:
/// every leg keeps its contract surrogate. Guards against a future "default"
/// that silently re-keys something it has no evidence for.
#[test]
fn an_empty_sac_map_leaves_every_leg_alone() {
    assert_eq!(
        contract_token_asset_id(XLM_SAC, &HashMap::new()),
        ids::contract_id(XLM_SAC),
    );
    assert_eq!(
        contract_token_asset_id(SOROBAN_TOKEN, &HashMap::new()),
        ids::contract_id(SOROBAN_TOKEN),
    );
}
