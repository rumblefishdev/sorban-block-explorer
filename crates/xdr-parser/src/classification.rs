//! Classification of Soroban contracts by WASM function signatures.
//!
//! Distinguishes SEP-0050-like NFT contracts from SEP-0041 fungible
//! token contracts. Motivated by task 0118 (audit finding F9): fungible
//! token transfers and NFT transfers emit events with the same topic
//! shape (`["transfer", Address(from), Address(to)]`) and both can
//! legitimately carry `i128` payloads — SEP-0041 as transfer amount,
//! some NFT contracts as token IDs. A payload-type heuristic cannot
//! tell them apart; the contract's WASM spec can.
//!
//! Phase 1 of task 0118 ships this pure classifier function only; the
//! Phase 2 integration (filtering at DB write time with per-worker
//! cache) is gated on task 0149's write-path rebuild landing.

use crate::types::ContractFunction;

/// Classification of a Soroban contract based on its public WASM
/// function set.
///
/// Consumed at ingest time to tag `soroban_contracts.contract_type`
/// and (via task 0118 Phase 2) to filter NFT-candidate events before
/// inserting into the `nfts` table. The `Other` variant means "no
/// usable WASM metadata yet" — the integration layer must never cache
/// this value and must re-query on next encounter, because an earlier
/// WASM upload may have arrived on a different worker in the meantime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractClassification {
    /// SEP-0050-style NFT contract. Discriminator: exposes any of
    /// `owner_of`, `token_uri`, `approve_for_all`, `get_approved`,
    /// or `is_approved_for_all`.
    Nft,
    /// SEP-0041 fungible token contract. Discriminator: exposes any
    /// of `decimals`, `allowance`, or `total_supply` without an NFT
    /// discriminator.
    Fungible,
    /// No usable classification yet — either empty metadata, a
    /// pre-standard contract, or a custom contract whose public
    /// surface carries none of the SEP discriminators.
    Other,
}

/// Classify a Soroban contract from its public WASM function list.
///
/// Input: the `functions` slice of [`crate::types::ContractFunction`]
/// as produced by [`crate::contract::extract_contract_interfaces`].
///
/// Pure function: no I/O, no allocation beyond the iterator closures.
///
/// # Discriminator source
///
/// Function-name sets below are taken from the OpenZeppelin Stellar
/// contracts library — the de-facto reference implementation used by
/// Stellar Developers documentation
/// (<https://developers.stellar.org/docs/build/smart-contracts/example-contracts/non-fungible-token>):
///
/// - `packages/tokens/src/non_fungible/mod.rs` defines the
///   `NonFungibleToken` trait.
/// - `packages/tokens/src/fungible/mod.rs` defines the `FungibleToken`
///   trait (SEP-0041).
///
/// Discriminators are the function names present on one trait and
/// absent from the other. Shared names (`transfer`, `transfer_from`,
/// `approve`, `balance`, `name`, `symbol`) are not discriminators.
///
/// # Rules
///
/// Precedence is top-down:
///
/// 1. Any of `owner_of` / `token_uri` / `approve_for_all` /
///    `get_approved` / `is_approved_for_all` present →
///    [`ContractClassification::Nft`]. These are NFT-unique per the
///    OpenZeppelin `NonFungibleToken` trait.
/// 2. Otherwise, any of `decimals` / `allowance` / `total_supply`
///    present → [`ContractClassification::Fungible`]. Fungible-unique
///    per the OpenZeppelin `FungibleToken` trait.
/// 3. Otherwise → [`ContractClassification::Other`].
///
/// **Dual-interface precedence** — a contract that implements both NFT
/// and fungible interfaces classifies as `Nft`. This is a deliberate
/// bias toward false positives (a fungible contract wrongly inserted
/// into `nfts` will be cleaned up by the Phase 3 post-backfill DELETE)
/// over false negatives (an NFT contract silently dropped on the
/// floor). See task 0118 Implementation section for the rationale.
///
/// # Notes
///
/// - `transfer`, `transfer_from`, `approve`, `balance`, `name`,
///   `symbol` appear on both traits and are **not** discriminators.
///   Even the `balance` return type differs (`u32` for NFT count,
///   `i128` for fungible amount), but discrimination by signature
///   (not just name) is left to a future refinement — current
///   classifier uses name-only matching.
/// - SAC (Stellar Asset Contract) contracts have no WASM; they are
///   tagged `'token'` at deploy time by the parser and never reach
///   this classifier. The integration layer (Phase 2) treats `'token'`
///   identically to `Fungible` for the NFT filter decision.
/// - Extending the enum with new variants is additive; the integration
///   filter should default to inserting (false-positive-safe) for any
///   classification it does not explicitly recognise.
pub fn classify_contract_from_wasm_spec(functions: &[ContractFunction]) -> ContractClassification {
    let has_nft_name_match = functions.iter().any(|f| {
        matches!(
            f.name.as_str(),
            "owner_of" | "token_uri" | "approve_for_all" | "get_approved" | "is_approved_for_all"
        )
    });
    if has_nft_name_match {
        return ContractClassification::Nft;
    }

    let has_fungible_name_match = functions
        .iter()
        .any(|f| matches!(f.name.as_str(), "decimals" | "allowance" | "total_supply"));
    if has_fungible_name_match {
        return ContractClassification::Fungible;
    }

    ContractClassification::Other
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ContractFunction;

    fn named(name: &str) -> ContractFunction {
        ContractFunction {
            name: name.to_string(),
            doc: String::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
        }
    }

    #[test]
    fn empty_functions_is_other() {
        assert_eq!(
            classify_contract_from_wasm_spec(&[]),
            ContractClassification::Other,
        );
    }

    #[test]
    fn nft_by_owner_of() {
        let functions = vec![named("owner_of"), named("transfer")];
        assert_eq!(
            classify_contract_from_wasm_spec(&functions),
            ContractClassification::Nft,
        );
    }

    #[test]
    fn nft_by_token_uri() {
        let functions = vec![named("token_uri"), named("transfer")];
        assert_eq!(
            classify_contract_from_wasm_spec(&functions),
            ContractClassification::Nft,
        );
    }

    #[test]
    fn fungible_openzeppelin_surface() {
        // Full OpenZeppelin FungibleToken trait (SEP-0041) surface.
        let functions = vec![
            named("total_supply"),
            named("balance"),
            named("allowance"),
            named("transfer"),
            named("transfer_from"),
            named("approve"),
            named("decimals"),
            named("name"),
            named("symbol"),
        ];
        assert_eq!(
            classify_contract_from_wasm_spec(&functions),
            ContractClassification::Fungible,
        );
    }

    #[test]
    fn fungible_by_total_supply_only() {
        // total_supply is present on Fungible but explicitly absent
        // from the NonFungibleToken trait.
        let functions = vec![named("total_supply"), named("transfer")];
        assert_eq!(
            classify_contract_from_wasm_spec(&functions),
            ContractClassification::Fungible,
        );
    }

    #[test]
    fn nft_openzeppelin_surface() {
        // Full OpenZeppelin NonFungibleToken trait surface.
        let functions = vec![
            named("balance"),
            named("owner_of"),
            named("transfer"),
            named("transfer_from"),
            named("approve"),
            named("approve_for_all"),
            named("get_approved"),
            named("is_approved_for_all"),
            named("name"),
            named("symbol"),
            named("token_uri"),
        ];
        assert_eq!(
            classify_contract_from_wasm_spec(&functions),
            ContractClassification::Nft,
        );
    }

    #[test]
    fn nft_by_approve_for_all_only() {
        // approve_for_all is NFT-unique (FungibleToken's approve has a
        // different signature but same name — not a discriminator).
        let functions = vec![named("approve_for_all"), named("transfer")];
        assert_eq!(
            classify_contract_from_wasm_spec(&functions),
            ContractClassification::Nft,
        );
    }

    #[test]
    fn nft_by_get_approved_only() {
        let functions = vec![named("get_approved"), named("transfer")];
        assert_eq!(
            classify_contract_from_wasm_spec(&functions),
            ContractClassification::Nft,
        );
    }

    #[test]
    fn nft_by_is_approved_for_all_only() {
        let functions = vec![named("is_approved_for_all"), named("transfer")];
        assert_eq!(
            classify_contract_from_wasm_spec(&functions),
            ContractClassification::Nft,
        );
    }

    #[test]
    fn fungible_by_allowance_only() {
        let functions = vec![named("transfer"), named("allowance")];
        assert_eq!(
            classify_contract_from_wasm_spec(&functions),
            ContractClassification::Fungible,
        );
    }

    #[test]
    fn dual_interface_nft_wins() {
        // Contract exposes both — precedence prefers false positives
        // (fungible misclassified as NFT, cleaned up post-backfill)
        // over false negatives (NFT dropped silently).
        let functions = vec![named("owner_of"), named("decimals"), named("transfer")];
        assert_eq!(
            classify_contract_from_wasm_spec(&functions),
            ContractClassification::Nft,
        );
    }

    #[test]
    fn unknown_surface_is_other() {
        // Custom contract, no standard discriminators.
        let functions = vec![named("execute"), named("init"), named("admin")];
        assert_eq!(
            classify_contract_from_wasm_spec(&functions),
            ContractClassification::Other,
        );
    }

    #[test]
    fn transfer_only_is_other() {
        // `transfer` is shared between SEP-0041 and SEP-0050 and is
        // therefore not a discriminator on its own.
        let functions = vec![named("transfer")];
        assert_eq!(
            classify_contract_from_wasm_spec(&functions),
            ContractClassification::Other,
        );
    }

    #[test]
    fn nft_precedence_with_token_uri_and_decimals() {
        // Dual-interface with different NFT discriminator.
        let functions = vec![named("token_uri"), named("decimals")];
        assert_eq!(
            classify_contract_from_wasm_spec(&functions),
            ContractClassification::Nft,
        );
    }

    #[test]
    fn additional_non_discriminators_do_not_shift_classification() {
        let functions = vec![
            named("owner_of"),
            named("some_internal_fn"),
            named("admin_helper"),
            named("transfer_from"),
        ];
        assert_eq!(
            classify_contract_from_wasm_spec(&functions),
            ContractClassification::Nft,
        );
    }
}
