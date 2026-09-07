//! Real-mainnet corpus (task 0393 cross-validation). Each fixture is a captured
//! `resultMetaXdr` (Soroban RPC `getTransaction`) for a tx whose ledger-value
//! output was cross-checked **1:1 against Horizon `/effects`** — including the
//! protocol-23 `contract_debited` / `contract_credited` effects, which Horizon
//! resolves SAC→native/USDC exactly like our `sac_classic` — on 2026-07-20. This
//! pins `ledger_balance_deltas` against the values an independent implementation
//! produced, on real data, across Native / Credit / SacWrapped / Bespoke.
//!
//! Gated on fixture presence (RPC retention is ~1 week; the fixtures persist).

use base64::Engine;
use stellar_xdr::{Limits, ReadXdr, TransactionMeta};
use xdr_parser::{LedgerAsset, LedgerDelta, ledger_balance_deltas};

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/corpus/");
const USDC_ISSUER: &str = "GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN";
const XLM_SAC: &str = "CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA";
const USDC_SAC: &str = "CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75";

fn deltas(fixture: &str) -> Option<Vec<LedgerDelta>> {
    let b64 = std::fs::read_to_string(format!("{DIR}{fixture}.b64")).ok()?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .expect("base64");
    let meta = TransactionMeta::from_xdr(bytes, Limits::none()).expect("decode meta");
    Some(ledger_balance_deltas(&meta))
}

fn credit(code: &str, issuer: &str) -> LedgerAsset {
    LedgerAsset::Credit {
        code: code.to_string(),
        issuer: issuer.to_string(),
    }
}

/// Assert exactly one `(account, asset, delta)` is present.
fn assert_has(ds: &[LedgerDelta], account: &str, asset: &LedgerAsset, delta: i128) {
    let n = ds
        .iter()
        .filter(|d| d.account == account && &d.asset == asset && d.delta == delta)
        .count();
    assert_eq!(
        n, 1,
        "want one {delta} on {asset:?} @ {account}; got {n} in {ds:#?}"
    );
}

#[test]
fn amm_swap_sac_contract_and_account_legs() {
    // AMM swap: account GCGPYY sends 92.4 USDC, receives 500.1793319 XLM; the pool
    // contract CA6PUJ holds both legs as SAC balances (ContractData). Horizon's
    // contract_debited 500.1793319 native / contract_credited 92.4000001 USDC matched
    // these 1:1 — so SacWrapped(XLM-SAC) resolves to native (nets with the account's
    // Native leg), SacWrapped(USDC-SAC) to USDC (nets with the Credit leg).
    let Some(ds) = deltas("amm_swap_sac") else {
        eprintln!("skip amm_swap_sac: fixture absent");
        return;
    };
    let acct = "GCGPYYULS3PYU3GN4NWU4JLXT4LMN53SINEFRXWDTBXHZYN2QHENMFCT";
    let pool = "CA6PUJLBYKZKUEKLZJMKBZLEKP2OTHANDEOWSFF44FTSYLKQPIICCJBE";
    assert_has(&ds, acct, &LedgerAsset::Native, 5_001_793_319);
    assert_has(&ds, acct, &credit("USDC", USDC_ISSUER), -924_000_001);
    assert_has(
        &ds,
        pool,
        &LedgerAsset::SacWrapped(XLM_SAC.to_string()),
        -5_001_793_319,
    );
    assert_has(
        &ds,
        pool,
        &LedgerAsset::SacWrapped(USDC_SAC.to_string()),
        924_000_001,
    );
    assert_eq!(ds.len(), 4, "exactly the 4 swap legs: {ds:#?}");
}

#[test]
fn bespoke_token_swap_bare_i128() {
    // Swap: account GAAXRLK sends 0.5 USDC, receives 4_685_128 of a bespoke token
    // (CCA2ZJP5, a bare-i128 ContractData balance held BY the account — bespoke tokens
    // use ContractData even for account holders, not trustlines). Confirms the Bespoke
    // branch on real data (the bare-i128 case that had no fixture before). Horizon
    // confirmed the 0.5 USDC leg; the bespoke leg has no classic representation.
    let Some(ds) = deltas("bespoke_swap") else {
        eprintln!("skip bespoke_swap: fixture absent");
        return;
    };
    let acct = "GAAXRLK32IILUEJTS7O3UP5PJ5EFH4HTCC4PSYZ6KT5L25523YFRWGSC";
    let bespoke = "CCA2ZJP5BVRXYTQH4FAGHCAUMRYCXVC4CRYC2NXHWMR7TIVX36U7F5HR";
    assert_has(&ds, acct, &credit("USDC", USDC_ISSUER), -5_000_000);
    assert_has(
        &ds,
        acct,
        &LedgerAsset::Bespoke(bespoke.to_string()),
        4_685_128,
    );
    assert!(
        ds.iter()
            .any(|d| matches!(&d.asset, LedgerAsset::SacWrapped(s) if s == USDC_SAC)),
        "contract-held USDC-SAC leg present: {ds:#?}"
    );
}

#[test]
fn path_payment_two_accounts_net_not_gross() {
    // Path payment (strict-receive) routed through a DEX + LPs: GBOMCO5L pays
    // 0.1181030 yXLM, receives 0.1189541 native; GA62O6VR is the crossed counterparty
    // (-0.1189541 native, +34.3542318 AFR). Horizon effects matched every account leg
    // 1:1. Native nets to 1_189_541 (one direction), not the gross 2×.
    let Some(ds) = deltas("path_payment") else {
        eprintln!("skip path_payment: fixture absent");
        return;
    };
    let a = "GBOMCO5L5YFTTOHUCISCHMM6HYSPCTA7YBBJDCKU3Y4RQUAPHBMIVFM3";
    let b = "GA62O6VRV2NLRCMP5WNFDWDDXMYHTZJLYVTINJGKYRXLUDADLX5LVWHO";
    let yxlm = "GARDNV3Q7YGT4AKSDF25LT32YSCCW4EV22Y2TV3I2PU2MMXJTEDL5T55";
    let afr = "GBX6YI45VU7WNAAKA3RBFDR3I3UKNFHTJPQ5F6KOOKSGYIAM4TRQN54W";
    assert_has(&ds, a, &LedgerAsset::Native, 1_189_541);
    assert_has(&ds, a, &credit("yXLM", yxlm), -1_181_030);
    assert_has(&ds, b, &LedgerAsset::Native, -1_189_541);
    assert_has(&ds, b, &credit("AFR", afr), 343_542_318);
    // Task 0540 / T03: the three classic pools the path crossed are holders now
    // (reserve deltas, `L…` StrKey). Before, value routed through a pool
    // netted to zero for the only holder the reader saw.
    let pool_1 = "LA2JELY7IYLFR2DERYJR2W2TVHF6PKD4QWDDE5M2VARLEMAYDARTCEXX";
    let pool_2 = "LA6FIZ2QNA4AXSTHBONIOV4IORFIYQISIAVMZFEVZRCBIWYHOFIRQEFA";
    let xtroop = "GARPXWTVB4QAGDZDCKV7ERN5GJTTMBSLOGPQPHYRVLYOW24CFB2SWCR5";
    let afrx = "GBDTAQDRSX3QOEAQQKYWRGOTFE5FHQTLIH5YAF2HGUY3PBKRXWDKBLHN";
    assert_has(&ds, pool_1, &credit("yXLM", yxlm), 1_181_030);
    assert_has(&ds, pool_1, &credit("XTROOP", xtroop), -428_489_993);
    assert_has(&ds, pool_2, &credit("XTROOP", xtroop), 428_489_993);
    assert_has(&ds, pool_2, &credit("AFRX", afrx), -21_667);
    let pool_3 = "LCSP2WMRNLPDFJ3NQ5RX7IJEZLGACTVJ4FE7AMRBUONGNHLC5XIJGDOD";
    assert_has(&ds, pool_3, &credit("AFRX", afrx), 21_667);
    assert_has(&ds, pool_3, &credit("AFR", afr), -343_542_318);
    assert_eq!(
        ds.len(),
        10,
        "two accounts × two assets + three pools × two reserves: {ds:#?}"
    );
}

#[test]
fn create_account_funds_new_account_native() {
    // create_account: funder GDDZBCMK debited 1009.1653521 XLM; new account GDKXBXYP
    // created at that balance (Created AccountEntry, before=0). 1:1 vs Horizon
    // account_created + account_debited.
    let Some(ds) = deltas("create_account") else {
        eprintln!("skip create_account");
        return;
    };
    let funder = "GDDZBCMKSNBV367AABKRCTQTZJEJSYZBIZHLQNS4TNLPQQTZSZJEH34G";
    let new = "GDKXBXYPZUQABKTQMGCYEBWU3PY2PM7NOLPAC2SPUAELLIZXWVHP6I7L";
    assert_has(&ds, funder, &LedgerAsset::Native, -10_091_653_521);
    assert_has(&ds, new, &LedgerAsset::Native, 10_091_653_521);
    assert_eq!(ds.len(), 2);
}

#[test]
fn account_merge_removes_source_moves_native() {
    // account_merge: source GCDBGSAJ removed (balance → 0), destination GCWB3T4M
    // credited it. Hits the Removed AccountEntry branch. 1:1 vs Horizon
    // account_removed + debited/credited.
    let Some(ds) = deltas("account_merge") else {
        eprintln!("skip account_merge");
        return;
    };
    let src = "GCDBGSAJKINKZCSKJKVRCXIKE4XCZGXJWSQTHMFFL5XXJTTY3MUHW7VS";
    let dst = "GCWB3T4MQEGXN6SYLRS35IRQSBA7P5GKYVGXNAJQXM6WIRVIIY6SXDJL";
    assert_has(&ds, src, &LedgerAsset::Native, -14_989_800);
    assert_has(&ds, dst, &LedgerAsset::Native, 14_989_800);
    assert_eq!(ds.len(), 2);
}

#[test]
fn failed_tx_moves_nothing() {
    // A failed transaction applies no operations; the fee is charged in the ledger's
    // separate feeProcessing phase (not TransactionMeta), so the reader sees zero
    // balance changes. 1:1 vs Horizon (no credited/debited effects).
    let Some(ds) = deltas("failed_tx") else {
        eprintln!("skip failed_tx");
        return;
    };
    assert!(ds.is_empty(), "a failed tx moves nothing, got {ds:#?}");
}

#[test]
fn soroban_mint_credits_receiver_issuer_absent() {
    // A Soroban invocation minting 7 classic assets to GASQATNX. The receiver's
    // trustlines go up (real ledger changes, captured); the ISSUER has no trustline in
    // its own asset (mints from nothing) → NO issuer-side delta, matching the ledger.
    // Receiver side is 1:1 vs Horizon account_credited (Horizon adds a virtual
    // issuer-debit effect that has no ledger entry).
    let Some(ds) = deltas("soroban_mint") else {
        eprintln!("skip soroban_mint");
        return;
    };
    let rx = "GASQATNXIQZMX5XEK2C5CXDB3XL3WCM3GQFXTEW72JMW2OXIM2MMI6GY";
    let iss = "GBAAVCWTP3FNMYDGOZDNLRC53YM7SKYWXTPONGWNV6L3JPL5KY7CMTXA";
    assert_has(&ds, rx, &credit("BTC", iss), 500_000);
    assert_has(&ds, rx, &credit("wXLM", iss), 50_000_000_000);
    assert_eq!(ds.len(), 7, "7 minted assets, receiver-side only: {ds:#?}");
    assert!(
        ds.iter().all(|d| d.account == rx && d.delta > 0),
        "all receiver credits, no issuer leg: {ds:#?}"
    );
}

#[test]
fn claimable_balance_nets_passthrough_and_sees_the_cb_holder() {
    // A complex create_claimable_balance tx. STARDUST flows GCPLNANL(issuer) →
    // GDHPJ6AC → GCUXCKRO (Horizon shows all 4 gross effects). Our reader NETS the
    // pass-through GDHPJ6AC to 0 (received then sent the same amount) and keeps only
    // GCUXCKRO's net +2_956_529.4 — matching the independent stellar-CLI decode 1:1.
    // The dSTARDUST that GCUXCKRO (its issuer) minted into a claimable balance
    // was the known **0413** understatement (an issuer has no trustline in its
    // own asset, and the reader skipped `ClaimableBalanceEntry`). Task 0540 /
    // T03 closed it: the claimable balance itself is the holder (`B…`).
    let Some(ds) = deltas("claimable_balance") else {
        eprintln!("skip claimable_balance");
        return;
    };
    let recv = "GCUXCKROP3B353YX2CGDAFB4ADKLGPFG4LLFU7CLHSELVOP4LAXDIYF7";
    let passthrough = "GDHPJ6ACPWRGR34VAHCHN775GLPVFCCT3NWGZWDKBNF36CPMIYQXG4S3";
    let stardust_iss = "GCPLNANLXTV2G6NPIRDPOLM5NAO5XH2IPYQBZCTTAKPNV4IRTU5X5B4S";
    assert_has(
        &ds,
        recv,
        &credit("STARDUST", stardust_iss),
        29_565_294_000_000,
    );
    assert!(
        !ds.iter().any(|d| d.account == passthrough),
        "pass-through account must net to 0 → dropped: {ds:#?}"
    );
    let cb = "BAALKCO6IRHEUHHXEXBQNI6ETZ3XPP5VMCJ475J5BQTIYAJ22VXMZPWH4M";
    assert_has(&ds, cb, &credit("dSTARDUST", recv), 52_222_151_490_373);
    assert_eq!(
        ds.len(),
        2,
        "the net-settled STARDUST leg and the claimable balance's dSTARDUST: {ds:#?}"
    );
}
