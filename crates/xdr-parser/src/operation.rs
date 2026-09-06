//! Operation extraction from transaction envelopes.
//!
//! Extracts per-operation structured data with type-specific JSON details.
//! INVOKE_HOST_FUNCTION operations get enriched extraction: contractId,
//! functionName, functionArgs (ScVal decoded), and returnValue.

use crate::envelope::{InnerTxRef, muxed_id, muxed_to_g_strkey};
use crate::scval::scval_to_typed_json;
use crate::types::ExtractedOperation;
use domain::OperationType;
use serde_json::{Value, json};
use stellar_xdr::*;

/// Extract all operations from a transaction envelope, with optional return
/// value from the transaction meta (for INVOKE_HOST_FUNCTION).
///
/// `tx_meta` is needed to extract the Soroban return value. Pass the
/// `TransactionMeta` from the processing result.
///
/// `op_results` is needed to extract the liquidity pools crossed by
/// path-payment ops (claim atoms live in the result, not the envelope —
/// task 0261). Pass `tx_op_results(&TransactionResult)`; index-aligned
/// with the envelope's operations. `None` degrades gracefully: details
/// simply lack `poolIds` / `claimedAtoms`.
pub fn extract_operations(
    envelope: &InnerTxRef<'_>,
    tx_meta: Option<&TransactionMeta>,
    op_results: Option<&[OperationResult]>,
    transaction_hash: &str,
    ledger_sequence: u32,
    tx_index: usize,
) -> Vec<ExtractedOperation> {
    let ops = match envelope {
        InnerTxRef::V0(tx) => tx.operations.as_slice(),
        InnerTxRef::V1(tx) => tx.operations.as_slice(),
    };

    let return_value = tx_meta.and_then(soroban_return_value);
    // Resolved once; the fallback issuer for `allow_trust` ops that inherit it.
    let tx_source = envelope.source_account();

    ops.iter()
        .enumerate()
        .map(|(i, op)| {
            // operation_index is 1-based to match Stellar ecosystem convention
            // (Horizon paging_token encodes op_app_order in low 12 bits, also
            // 1-based). Surfaces as user-facing `application_order` in
            // `XdrOperationDto`. See task 0172 / ADR 0028.
            let op_index = i + 1;
            let source_account = op.source_account.as_ref().map(muxed_to_g_strkey);
            // Task 0359: every asset the op touches — body + same-op meta changes
            // (no result grain; see `asset_appearances` docs). `op_source` (op
            // override or tx source) is the AllowTrust issuer; it borrows
            // `source_account`, released before the move below.
            let op_source = source_account.as_deref().unwrap_or(&tx_source);
            let op_changes = op_meta_changes(tx_meta, i);
            let asset_appearances =
                crate::asset_appearances::emit_asset_appearances(&op.body, op_source, op_changes);
            // Shared by details (poolIds/claimedAtoms) and counterparties
            // (crossed-offer sellers); both read the same per-op result.
            let op_result = op_results.and_then(|rs| rs.get(i));
            let counterparties =
                crate::op_participants::extract_counterparties(&op.body, op_result);
            let (op_type, details) = extract_op_details(
                &op.body,
                return_value.as_ref(),
                op_result,
                op_changes,
                ledger_sequence,
                tx_index,
                op_index,
            );
            ExtractedOperation {
                transaction_hash: transaction_hash.to_string(),
                operation_index: u32::try_from(op_index)
                    .expect("operation index does not fit into u32"),
                op_type,
                source_account,
                details,
                asset_appearances,
                counterparties,
                source_muxed_id: op.source_account.as_ref().and_then(muxed_id),
                destination_muxed_id: destination_muxed_id(&op.body),
            }
        })
        .collect()
}

/// The multiplexing id of an operation's destination when it is an `M…`
/// address. Only the four classic operations whose destination is a
/// `MuxedAccount` can carry one (CAP-27); everything else → `None`. The `G…`
/// half is what `details.destination` already records.
fn destination_muxed_id(body: &OperationBody) -> Option<u64> {
    match body {
        OperationBody::Payment(op) => muxed_id(&op.destination),
        OperationBody::PathPaymentStrictReceive(op) => muxed_id(&op.destination),
        OperationBody::PathPaymentStrictSend(op) => muxed_id(&op.destination),
        OperationBody::AccountMerge(destination) => muxed_id(destination),
        _ => None,
    }
}

/// Per-operation results of a **successful** transaction.
///
/// Unwraps the fee-bump nesting (`TxFeeBumpInnerSuccess` carries the inner
/// transaction's per-op results one level down). Returns `None` for any failed
/// transaction: when a transaction fails, every operation is rolled back, yet
/// the op-level result of an operation that executed before the failing one
/// still shows `Success` (with claim atoms) for crossings that never
/// committed. Gating on tx success here keeps those phantom crossings out of
/// `pool_ids` and the downstream `gross_volume_a` (task 0261). Claim atoms are
/// the sole consumer of this accessor, so success-only is the correct scope.
pub fn tx_op_results(result: &TransactionResult) -> Option<&[OperationResult]> {
    match &result.result {
        TransactionResultResult::TxSuccess(ops) => Some(ops.as_slice()),
        TransactionResultResult::TxFeeBumpInnerSuccess(pair) => match &pair.result.result {
            InnerTransactionResultResult::TxSuccess(ops) => Some(ops.as_slice()),
            _ => None,
        },
        _ => None,
    }
}

/// Per-operation results of ANY applied transaction — success or failed.
///
/// Companion to [`tx_op_results`], which stays success-only because its sole
/// consumer is claim atoms (task 0261 — rolled-back crossings must not reach
/// `poolIds`). `TxFailed` carries the same per-op array as `TxSuccess`, and
/// the failing operation's code IS the transaction's fail reason (task 0352),
/// so this accessor unwraps all four applied arms. `None` only for
/// validation-level failures (`TxBadSeq`, `TxInsufficientFee`, …), where no
/// operation was ever attempted and no per-op array exists.
pub fn tx_op_results_any(result: &TransactionResult) -> Option<&[OperationResult]> {
    use TransactionResultResult::*;
    match &result.result {
        TxSuccess(ops) | TxFailed(ops) => Some(ops.as_slice()),
        TxFeeBumpInnerSuccess(pair) | TxFeeBumpInnerFailed(pair) => match &pair.result.result {
            InnerTransactionResultResult::TxSuccess(ops)
            | InnerTransactionResultResult::TxFailed(ops) => Some(ops.as_slice()),
            _ => None,
        },
        _ => None,
    }
}

/// The result code of a single operation, named by the XDR library's own
/// variant names — no hand-rolled name table (the 0431 lesson).
///
/// `OpInner` unwraps to the operation-specific result variant (`"Success"`,
/// `"LowReserve"`, `"Trapped"`, …); the operation-level rejections keep their
/// `Op…` names (`"OpNoAccount"`, `"OpBadAuth"`, …). Task 0352: the failing
/// operation's code is what the fail-reason banner shows.
pub fn op_result_code(op_result: &OperationResult) -> &'static str {
    match op_result {
        OperationResult::OpInner(tr) => {
            use OperationResultTr::*;
            match tr {
                CreateAccount(r) => r.name(),
                Payment(r) => r.name(),
                PathPaymentStrictReceive(r) => r.name(),
                ManageSellOffer(r) | CreatePassiveSellOffer(r) => r.name(),
                SetOptions(r) => r.name(),
                ChangeTrust(r) => r.name(),
                AllowTrust(r) => r.name(),
                AccountMerge(r) => r.name(),
                Inflation(r) => r.name(),
                ManageData(r) => r.name(),
                BumpSequence(r) => r.name(),
                ManageBuyOffer(r) => r.name(),
                PathPaymentStrictSend(r) => r.name(),
                CreateClaimableBalance(r) => r.name(),
                ClaimClaimableBalance(r) => r.name(),
                BeginSponsoringFutureReserves(r) => r.name(),
                EndSponsoringFutureReserves(r) => r.name(),
                RevokeSponsorship(r) => r.name(),
                Clawback(r) => r.name(),
                ClawbackClaimableBalance(r) => r.name(),
                SetTrustLineFlags(r) => r.name(),
                LiquidityPoolDeposit(r) => r.name(),
                LiquidityPoolWithdraw(r) => r.name(),
                InvokeHostFunction(r) => r.name(),
                ExtendFootprintTtl(r) => r.name(),
                RestoreFootprint(r) => r.name(),
            }
        }
        other => other.name(),
    }
}

/// All claim atoms — order-book AND liquidity-pool — crossed by a successful op
/// result, in XDR vector order.
///
/// Covers every atom-bearing op kind: both path-payment variants (`offers`)
/// and all three offer ops — `ManageSellOffer` / `ManageBuyOffer` /
/// `CreatePassiveSellOffer` (`ManageOfferSuccessResult.offers_claimed`), which
/// can fill against an AMM (CAP-38 unified order-book + pool exchange). Empty
/// for any non-success branch and non-atom-bearing ops. This is the "unified
/// claim-atom extractor" of the 0261 decision — one generic pass captures every
/// crossing, with no second historical pass. Consumers: [`claim_lp_atoms`]
/// (pool-only, `poolIds` / `gross_volume_a`) and [`extract_counterparties`]
/// (crossed-offer sellers, task 0359 F-C).
pub(crate) fn claim_atoms(op_result: &OperationResult) -> &[ClaimAtom] {
    use OperationResultTr::*;
    match op_result {
        OperationResult::OpInner(PathPaymentStrictSend(PathPaymentStrictSendResult::Success(
            s,
        ))) => s.offers.as_slice(),
        OperationResult::OpInner(PathPaymentStrictReceive(
            PathPaymentStrictReceiveResult::Success(s),
        )) => s.offers.as_slice(),
        OperationResult::OpInner(
            ManageSellOffer(ManageSellOfferResult::Success(m))
            | CreatePassiveSellOffer(ManageSellOfferResult::Success(m)),
        ) => m.offers_claimed.as_slice(),
        OperationResult::OpInner(ManageBuyOffer(ManageBuyOfferResult::Success(m))) => {
            m.offers_claimed.as_slice()
        }
        _ => &[],
    }
}

/// Liquidity-pool claim atoms only — the `poolIds` / `gross_volume_a` consumer
/// (task 0261). Order-book fills are excluded (they carry no pool id).
fn claim_lp_atoms(op_result: &OperationResult) -> impl Iterator<Item = &ClaimLiquidityAtom> {
    claim_atoms(op_result).iter().filter_map(|a| match a {
        ClaimAtom::LiquidityPool(lp) => Some(lp),
        _ => None,
    })
}

/// Append `poolIds` + `claimedAtoms` to an op's details when its result shows
/// liquidity-pool crossings (path payments and offers; see [`claim_lp_atoms`]).
/// `poolIds` is the deduped crossed-pool list (first-crossing order);
/// `claimedAtoms` keeps every LP fill with its amounts so `gross_volume_a` per
/// (pool, ledger) can be computed downstream without a second parse pass
/// (tasks 0247/0266/0199). No-op for ops with no LP atoms, so it is safe to
/// call unconditionally for every op.
///
/// Each atom also carries `amountA` — the fill amount on the pool's **canonical
/// asset A** side. A pool's `assetA` is by definition the canonically-smaller
/// of its two assets (`assetA < assetB`, Stellar pool-param rule), and an
/// atom's `{asset_sold, asset_bought}` are exactly the pool's two assets, so
/// `amountA` is the amount on `min(asset_sold, asset_bought)` (XDR `Asset`
/// ordering = type, then code, then issuer) — no pool-definition lookup
/// needed. This is the `gross_volume_a` per-atom contribution (task 0266
/// Phase 2 / 0279): the worker sums `amountA` per `(pool, ledger)`.
fn append_pool_claims(details: &mut Value, op_result: Option<&OperationResult>) {
    let Some(op_result) = op_result else { return };
    let Value::Object(map) = details else { return };

    let mut pool_ids: Vec<String> = Vec::new();
    let mut claimed: Vec<Value> = Vec::new();
    for atom in claim_lp_atoms(op_result) {
        let id = hex::encode(atom.liquidity_pool_id.0.as_slice());
        // Canonical asset-A side: the smaller of the two assets carries A.
        let amount_a = if atom.asset_sold <= atom.asset_bought {
            atom.amount_sold
        } else {
            atom.amount_bought
        };
        claimed.push(json!({
            "poolId": &id,
            "assetSold": format_asset(&atom.asset_sold),
            "amountSold": atom.amount_sold,
            "assetBought": format_asset(&atom.asset_bought),
            "amountBought": atom.amount_bought,
            "amountA": amount_a,
        }));
        if !pool_ids.contains(&id) {
            pool_ids.push(id);
        }
    }
    if claimed.is_empty() {
        return;
    }
    map.insert("poolIds".into(), Value::from(pool_ids));
    map.insert("claimedAtoms".into(), Value::from(claimed));
}

/// Extract the Soroban return value from TransactionMeta, if present.
fn soroban_return_value(meta: &TransactionMeta) -> Option<ScVal> {
    match meta {
        TransactionMeta::V3(v3) => v3.soroban_meta.as_ref().map(|m| m.return_value.clone()),
        TransactionMeta::V4(v4) => v4
            .soroban_meta
            .as_ref()
            .and_then(|m| m.return_value.clone()),
        _ => None,
    }
}

/// The ledger changes of operation `op_idx` (0-based), index-aligned with the
/// envelope's operations. Task 0359 uses these for the meta grain — the assets
/// of ops that name a claimable balance / LP only by id. Empty for the out-of-
/// range index.
///
/// The `TransactionMeta` match is EXHAUSTIVE (no `_`) on purpose: a future
/// variant (e.g. a Protocol-24 `V5`) must break the build HERE, not silently
/// yield zero meta changes and drop every claim-CB / LP asset on those ledgers
/// (the meta grain is their ONLY source). The legacy pre-Soroban metas (V0–V2)
/// map to `&[]` because the 0359 backfill window starts at the Soroban go-live
/// (ledger 50,457,424 / Protocol 20 / `V3`+), so V0..V2 never occur in the ingest
/// window. NB this is a WINDOW guarantee, not a protocol one: claimable balances
/// (Protocol 15) and liquidity pools (Protocol 18) predate `V3`, so their CB/LP
/// ops on P15..P19 ledgers live in `V2` meta -- a pre-P20 ingest would need `V2`
/// handling here or it would silently drop those meta-grain assets.
fn op_meta_changes(tx_meta: Option<&TransactionMeta>, op_idx: usize) -> &[LedgerEntryChange] {
    match tx_meta {
        None | Some(TransactionMeta::V0(_) | TransactionMeta::V1(_) | TransactionMeta::V2(_)) => {
            &[]
        }
        Some(TransactionMeta::V3(v3)) => v3
            .operations
            .get(op_idx)
            .map(|m| m.changes.as_slice())
            .unwrap_or(&[]),
        Some(TransactionMeta::V4(v4)) => v4
            .operations
            .get(op_idx)
            .map(|m| m.changes.as_slice())
            .unwrap_or(&[]),
    }
}

/// Extract operation type discriminator and details JSON for a single
/// operation. Matches the XDR body variant; the resulting `OperationType`
/// casts to SMALLINT via `#[repr(i16)]` with zero lookup cost.
fn extract_op_details(
    body: &OperationBody,
    return_value: Option<&ScVal>,
    op_result: Option<&OperationResult>,
    op_changes: &[LedgerEntryChange],
    _ledger_sequence: u32,
    _tx_index: usize,
    _op_index: usize,
) -> (OperationType, Value) {
    // Per-op body → (type, details). `append_pool_claims` runs once after the
    // match: it no-ops unless the op result carries LP claim atoms, so path
    // payments AND offers crossing a pool (task 0261) get `poolIds`/
    // `claimedAtoms` without per-arm plumbing.
    let (op_type, mut details) = match body {
        OperationBody::CreateAccount(op) => (
            OperationType::CreateAccount,
            json!({
                "destination": op.destination.0.to_string(),
                "startingBalance": op.starting_balance,
            }),
        ),
        OperationBody::Payment(op) => (
            OperationType::Payment,
            json!({
                "destination": muxed_to_g_strkey(&op.destination),
                "asset": format_asset(&op.asset),
                "amount": op.amount,
            }),
        ),
        OperationBody::PathPaymentStrictReceive(op) => (
            OperationType::PathPaymentStrictReceive,
            json!({
                "sendAsset": format_asset(&op.send_asset),
                "sendMax": op.send_max,
                "destination": muxed_to_g_strkey(&op.destination),
                "destAsset": format_asset(&op.dest_asset),
                "destAmount": op.dest_amount,
                "path": op.path.iter().map(format_asset).collect::<Vec<_>>(),
            }),
        ),
        OperationBody::PathPaymentStrictSend(op) => (
            OperationType::PathPaymentStrictSend,
            json!({
                "sendAsset": format_asset(&op.send_asset),
                "sendAmount": op.send_amount,
                "destination": muxed_to_g_strkey(&op.destination),
                "destAsset": format_asset(&op.dest_asset),
                "destMin": op.dest_min,
                "path": op.path.iter().map(format_asset).collect::<Vec<_>>(),
            }),
        ),
        OperationBody::ManageSellOffer(op) => (
            OperationType::ManageSellOffer,
            json!({
                "selling": format_asset(&op.selling),
                "buying": format_asset(&op.buying),
                "amount": op.amount,
                "price": format_price(&op.price),
                "offerId": op.offer_id,
            }),
        ),
        OperationBody::ManageBuyOffer(op) => (
            OperationType::ManageBuyOffer,
            json!({
                "selling": format_asset(&op.selling),
                "buying": format_asset(&op.buying),
                "buyAmount": op.buy_amount,
                "price": format_price(&op.price),
                "offerId": op.offer_id,
            }),
        ),
        OperationBody::CreatePassiveSellOffer(op) => (
            OperationType::CreatePassiveSellOffer,
            json!({
                "selling": format_asset(&op.selling),
                "buying": format_asset(&op.buying),
                "amount": op.amount,
                "price": format_price(&op.price),
            }),
        ),
        OperationBody::SetOptions(op) => {
            let mut details = serde_json::Map::new();
            if let Some(ref dest) = op.inflation_dest {
                details.insert("inflationDest".into(), json!(dest.0.to_string()));
            }
            if let Some(flags) = op.clear_flags {
                details.insert("clearFlags".into(), json!(flags));
            }
            if let Some(flags) = op.set_flags {
                details.insert("setFlags".into(), json!(flags));
            }
            if let Some(w) = op.master_weight {
                details.insert("masterWeight".into(), json!(w));
            }
            if let Some(t) = op.low_threshold {
                details.insert("lowThreshold".into(), json!(t));
            }
            if let Some(t) = op.med_threshold {
                details.insert("medThreshold".into(), json!(t));
            }
            if let Some(t) = op.high_threshold {
                details.insert("highThreshold".into(), json!(t));
            }
            if let Some(ref domain) = op.home_domain {
                let s = std::str::from_utf8(domain.as_vec()).unwrap_or("<invalid-utf8>");
                details.insert("homeDomain".into(), json!(s));
            }
            if let Some(ref signer) = op.signer {
                details.insert("signerKey".into(), json!(signer.key.to_string()));
                details.insert("signerWeight".into(), json!(signer.weight));
            }
            (OperationType::SetOptions, Value::Object(details))
        }
        OperationBody::ChangeTrust(op) => (
            OperationType::ChangeTrust,
            json!({
                "asset": format_change_trust_asset(&op.line),
                "limit": op.limit,
            }),
        ),
        OperationBody::AllowTrust(op) => (
            OperationType::AllowTrust,
            json!({
                "trustor": op.trustor.0.to_string(),
                "asset": format_asset_code(&op.asset),
                "authorize": op.authorize,
            }),
        ),
        OperationBody::AccountMerge(destination) => (
            OperationType::AccountMerge,
            json!({
                "destination": muxed_to_g_strkey(destination),
            }),
        ),
        OperationBody::Inflation => (OperationType::Inflation, json!({})),
        OperationBody::ManageData(op) => {
            let name = std::str::from_utf8(op.data_name.as_vec()).unwrap_or("<invalid-utf8>");
            let value = op.data_value.as_ref().map(|v| {
                base64::Engine::encode(&base64::engine::general_purpose::STANDARD, v.as_slice())
            });
            (
                OperationType::ManageData,
                json!({
                    "name": name,
                    "value": value,
                }),
            )
        }
        OperationBody::BumpSequence(op) => (
            OperationType::BumpSequence,
            json!({
                "bumpTo": op.bump_to.0,
            }),
        ),
        OperationBody::CreateClaimableBalance(op) => (
            OperationType::CreateClaimableBalance,
            json!({
                "asset": format_asset(&op.asset),
                "amount": op.amount,
                // Task 0460 #16: the full claimant vec — the addresses are
                // the point of the operation. One shape, no summary count
                // beside it (derive length where needed).
                "claimants": op.claimants.iter().map(claimant_json).collect::<Vec<_>>(),
            }),
        ),
        OperationBody::ClaimClaimableBalance(op) => (
            OperationType::ClaimClaimableBalance,
            cb_details(op_changes, &op.balance_id),
        ),
        OperationBody::BeginSponsoringFutureReserves(op) => (
            OperationType::BeginSponsoringFutureReserves,
            json!({
                "sponsoredId": op.sponsored_id.0.to_string(),
            }),
        ),
        OperationBody::EndSponsoringFutureReserves => {
            (OperationType::EndSponsoringFutureReserves, json!({}))
        }
        OperationBody::RevokeSponsorship(op) => {
            let details = match op {
                RevokeSponsorshipOp::LedgerEntry(key) => json!({
                    "kind": "ledgerEntry",
                    "ledgerKeyType": key.name(),
                }),
                RevokeSponsorshipOp::Signer(s) => json!({
                    "kind": "signer",
                    "accountId": s.account_id.0.to_string(),
                    "signerKey": s.signer_key.to_string(),
                }),
            };
            (OperationType::RevokeSponsorship, details)
        }
        OperationBody::Clawback(op) => (
            OperationType::Clawback,
            json!({
                "asset": format_asset(&op.asset),
                "from": muxed_to_g_strkey(&op.from),
                "amount": op.amount,
            }),
        ),
        OperationBody::ClawbackClaimableBalance(op) => (
            OperationType::ClawbackClaimableBalance,
            cb_details(op_changes, &op.balance_id),
        ),
        OperationBody::SetTrustLineFlags(op) => (
            OperationType::SetTrustLineFlags,
            json!({
                "trustor": op.trustor.0.to_string(),
                "asset": format_asset(&op.asset),
                "clearFlags": op.clear_flags,
                "setFlags": op.set_flags,
            }),
        ),
        OperationBody::LiquidityPoolDeposit(op) => {
            let mut details = json!({
                "liquidityPoolId": hex::encode(op.liquidity_pool_id.0.as_slice()),
                "maxAmountA": op.max_amount_a,
                "maxAmountB": op.max_amount_b,
                "minPrice": format_price(&op.min_price),
                "maxPrice": format_price(&op.max_price),
            });
            if let Some(delta) = pool_delta_details(op_changes, &op.liquidity_pool_id) {
                details["poolDelta"] = delta;
            }
            (OperationType::LiquidityPoolDeposit, details)
        }
        OperationBody::LiquidityPoolWithdraw(op) => {
            let mut details = json!({
                "liquidityPoolId": hex::encode(op.liquidity_pool_id.0.as_slice()),
                "amount": op.amount,
                "minAmountA": op.min_amount_a,
                "minAmountB": op.min_amount_b,
            });
            if let Some(delta) = pool_delta_details(op_changes, &op.liquidity_pool_id) {
                details["poolDelta"] = delta;
            }
            (OperationType::LiquidityPoolWithdraw, details)
        }
        OperationBody::InvokeHostFunction(op) => {
            let details = extract_invoke_host_function(op, return_value);
            (OperationType::InvokeHostFunction, details)
        }
        OperationBody::ExtendFootprintTtl(op) => (
            OperationType::ExtendFootprintTtl,
            json!({
                "extendTo": op.extend_to,
            }),
        ),
        OperationBody::RestoreFootprint(_) => (OperationType::RestoreFootprint, json!({})),
    };
    append_pool_claims(&mut details, op_result);
    (op_type, details)
}

/// Extract enriched details for INVOKE_HOST_FUNCTION operations.
fn extract_invoke_host_function(op: &InvokeHostFunctionOp, return_value: Option<&ScVal>) -> Value {
    match &op.host_function {
        HostFunction::InvokeContract(args) => {
            let contract_id = args.contract_address.to_string();
            let function_name =
                std::str::from_utf8(args.function_name.as_vec()).unwrap_or("<invalid-utf8>");
            let function_args: Vec<Value> = args.args.iter().map(scval_to_typed_json).collect();
            let ret = return_value.map(scval_to_typed_json);
            json!({
                "hostFunctionType": "invokeContract",
                "contractId": contract_id,
                "functionName": function_name,
                "functionArgs": function_args,
                "returnValue": ret,
            })
        }
        HostFunction::CreateContract(args) => {
            json!({
                "hostFunctionType": "createContract",
                "executable": format_contract_executable(&args.executable),
            })
        }
        HostFunction::UploadContractWasm(wasm) => {
            json!({
                "hostFunctionType": "uploadContractWasm",
                "wasmLength": wasm.len(),
            })
        }
        HostFunction::CreateContractV2(args) => {
            json!({
                "hostFunctionType": "createContractV2",
                "executable": format_contract_executable(&args.executable),
                "constructorArgs": args.constructor_args.iter().map(scval_to_typed_json).collect::<Vec<_>>(),
            })
        }
    }
}

// --- Formatting helpers ---

fn format_asset(asset: &Asset) -> Value {
    match asset {
        Asset::Native => json!("native"),
        Asset::CreditAlphanum4(a) => {
            let code = crate::asset_code::asset_code_str(a.asset_code.as_slice());
            json!(format!("{}:{}", code, a.issuer.0.to_string()))
        }
        Asset::CreditAlphanum12(a) => {
            let code = crate::asset_code::asset_code_str(a.asset_code.as_slice());
            json!(format!("{}:{}", code, a.issuer.0.to_string()))
        }
    }
}

fn format_change_trust_asset(asset: &ChangeTrustAsset) -> Value {
    match asset {
        ChangeTrustAsset::Native => json!("native"),
        ChangeTrustAsset::CreditAlphanum4(a) => {
            let code = crate::asset_code::asset_code_str(a.asset_code.as_slice());
            json!(format!("{}:{}", code, a.issuer.0.to_string()))
        }
        ChangeTrustAsset::CreditAlphanum12(a) => {
            let code = crate::asset_code::asset_code_str(a.asset_code.as_slice());
            json!(format!("{}:{}", code, a.issuer.0.to_string()))
        }
        ChangeTrustAsset::PoolShare(params) => {
            json!({ "type": "liquidityPool", "params": params.name() })
        }
    }
}

fn format_asset_code(code: &AssetCode) -> Value {
    let s = crate::asset_code::asset_code_str(crate::asset_code::asset_code_bytes(code));
    json!(s)
}

fn format_price(price: &Price) -> Value {
    json!({ "n": price.n, "d": price.d })
}

fn format_claimable_balance_id(id: &ClaimableBalanceId) -> Value {
    match id {
        ClaimableBalanceId::ClaimableBalanceIdTypeV0(hash) => {
            json!(hex::encode(hash.0))
        }
    }
}

/// Claimant → `{destination, predicate}` (task 0460 #16).
fn claimant_json(claimant: &Claimant) -> Value {
    let Claimant::ClaimantTypeV0(v0) = claimant;
    json!({
        "destination": v0.destination.0.to_string(),
        "predicate": claim_predicate_json(&v0.predicate),
    })
}

/// The full recursive claim predicate as tagged JSON — no lossy summary;
/// `and`/`or` carry up to 2 sub-predicates by XDR definition.
fn claim_predicate_json(predicate: &ClaimPredicate) -> Value {
    match predicate {
        ClaimPredicate::Unconditional => json!({ "type": "unconditional" }),
        ClaimPredicate::And(ps) => json!({
            "type": "and",
            "predicates": ps.iter().map(claim_predicate_json).collect::<Vec<_>>(),
        }),
        ClaimPredicate::Or(ps) => json!({
            "type": "or",
            "predicates": ps.iter().map(claim_predicate_json).collect::<Vec<_>>(),
        }),
        ClaimPredicate::Not(inner) => json!({
            "type": "not",
            "predicate": inner.as_deref().map(claim_predicate_json),
        }),
        ClaimPredicate::BeforeAbsoluteTime(t) => json!({
            "type": "beforeAbsoluteTime",
            "timePoint": t,
        }),
        ClaimPredicate::BeforeRelativeTime(secs) => json!({
            "type": "beforeRelativeTime",
            "seconds": secs,
        }),
    }
}

/// Details for claim/clawback-claimable-balance — shared by both arms. The
/// body carries only the id; the asset + amount live in the same-op ledger
/// entry (task 0453 D8). Keys are optional — absent when the meta lacks the
/// entry, never guessed.
fn cb_details(op_changes: &[LedgerEntryChange], balance_id: &ClaimableBalanceId) -> Value {
    let mut d = json!({
        "balanceId": format_claimable_balance_id(balance_id),
    });
    if let Some((asset, amount)) =
        crate::asset_appearances::claimed_cb_asset_amount(op_changes, balance_id)
    {
        d["asset"] = json!(format_asset(&asset));
        d["amount"] = json!(amount);
    }
    d
}

/// The constant-product body of `entry`, when it is the pool `pool_id`.
/// Matching on the id (not the first pool entry) mirrors `lp_pool_assets`.
fn pool_constant_product<'a>(
    entry: &'a LedgerEntry,
    pool_id: &PoolId,
) -> Option<&'a LiquidityPoolEntryConstantProduct> {
    match &entry.data {
        LedgerEntryData::LiquidityPool(lp) if &lp.liquidity_pool_id == pool_id => {
            let LiquidityPoolEntryBody::LiquidityPoolConstantProduct(cp) = &lp.body;
            Some(cp)
        }
        _ => None,
    }
}

/// What an LP deposit / withdraw actually moved, read from the operation's OWN
/// ledger changes (task 0279).
///
/// The body carries only the caller's bounds — `maxAmountA`/`maxAmountB` on a
/// deposit, `amount` + `minAmountA`/`minAmountB` on a withdrawal — never what
/// executed. The pool entry's before/after images are the only record of that,
/// which is why these ops have no claim atoms to read instead. Same contract as
/// [`cb_details`]: `None` when the meta lacks the entry, never guessed.
///
/// `amountA` / `amountB` are AFTER MINUS BEFORE, i.e. **signed from the pool's
/// side** — a deposit reads `+/+`, a withdrawal `-/-` — which is the sign the
/// claim-atom consumers already use for trades, so one downstream shape covers
/// all three event kinds (`lp_operation_amounts`). The boundary cases fall out
/// of the same subtraction: a pool created by its first deposit has no `state`
/// pre-image (before = 0) and one emptied by its last withdrawal is `Removed`
/// with no post-image (after = 0).
///
/// Coverage follows [`op_meta_changes`]: V0–V2 metas yield no changes, so this
/// is silent on pre-Protocol-20 ledgers — outside the ingest window, which
/// starts at the Soroban go-live.
fn pool_delta_details(op_changes: &[LedgerEntryChange], pool_id: &PoolId) -> Option<Value> {
    let mut before = None;
    let mut after = None;
    let mut removed = false;
    for change in op_changes {
        match change {
            // FIRST match wins for the pre-image (`before.or(new)`), LAST for
            // the post-image (`new.or(after)`) — the two ends of the op. The
            // asymmetry only shows when one op touches a pool repeatedly, as a
            // path payment does under CAP-38 interleaved matching (a
            // State/Updated pair per fill): taking the last `State` there would
            // measure the final fill instead of the whole operation. Deposits
            // and withdrawals touch it once, so today both readings agree.
            LedgerEntryChange::State(e) => before = before.or(pool_constant_product(e, pool_id)),
            LedgerEntryChange::Created(e)
            | LedgerEntryChange::Updated(e)
            | LedgerEntryChange::Restored(e) => after = pool_constant_product(e, pool_id).or(after),
            LedgerEntryChange::Removed(LedgerKey::LiquidityPool(k)) => {
                removed |= &k.liquidity_pool_id == pool_id;
            }
            LedgerEntryChange::Removed(_) => {}
        }
    }

    // Either image identifies the pool's two assets; they never change.
    let params = &after.or(before)?.params;
    let (before_a, before_b) = before.map_or((0, 0), |cp| (cp.reserve_a, cp.reserve_b));
    let (after_a, after_b) = match (removed, after) {
        (true, _) => (0, 0),
        (false, Some(cp)) => (cp.reserve_a, cp.reserve_b),
        // A pre-image with nothing after it means this op did not move the
        // pool (a `state`-only read). Nothing to report.
        (false, None) => return None,
    };

    let (delta_a, delta_b) = (after_a - before_a, after_b - before_b);
    if delta_a == 0 && delta_b == 0 {
        return None;
    }
    Some(json!({
        "poolId": hex::encode(pool_id.0.as_slice()),
        "assetA": format_asset(&params.asset_a),
        "amountA": delta_a,
        "assetB": format_asset(&params.asset_b),
        "amountB": delta_b,
    }))
}

fn format_contract_executable(exec: &ContractExecutable) -> Value {
    match exec {
        ContractExecutable::Wasm(hash) => json!({ "type": "wasm", "hash": hex::encode(hash.0) }),
        ContractExecutable::StellarAsset => json!({ "type": "stellar_asset" }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool_entry(reserve_a: i64, reserve_b: i64) -> LedgerEntry {
        LedgerEntry {
            last_modified_ledger_seq: 100,
            data: LedgerEntryData::LiquidityPool(LiquidityPoolEntry {
                liquidity_pool_id: PoolId(Hash([0x44; 32])),
                body: LiquidityPoolEntryBody::LiquidityPoolConstantProduct(
                    LiquidityPoolEntryConstantProduct {
                        params: LiquidityPoolConstantProductParameters {
                            asset_a: Asset::Native,
                            asset_b: Asset::CreditAlphanum4(AlphaNum4 {
                                asset_code: AssetCode4(*b"USDC"),
                                issuer: AccountId(PublicKey::PublicKeyTypeEd25519(Uint256(
                                    [0x05; 32],
                                ))),
                            }),
                            fee: 30,
                        },
                        reserve_a,
                        reserve_b,
                        total_pool_shares: 500,
                        pool_shares_trust_line_count: 3,
                    },
                ),
            }),
            ext: LedgerEntryExt::V0,
        }
    }

    /// A deposit adds to both reserves and a withdrawal takes from both, so the
    /// after-minus-before deltas must come out `+/+` and `-/-` — the sign
    /// convention the trade path already uses.
    #[test]
    fn pool_delta_signs_deposits_positive_and_withdrawals_negative() {
        let pool_id = PoolId(Hash([0x44; 32]));
        let deposit = vec![
            LedgerEntryChange::State(pool_entry(1_000, 2_000)),
            LedgerEntryChange::Updated(pool_entry(1_500, 3_000)),
        ];
        let delta = pool_delta_details(&deposit, &pool_id).expect("deposit delta");
        assert_eq!(delta["amountA"], json!(500));
        assert_eq!(delta["amountB"], json!(1_000));
        assert_eq!(delta["assetA"], json!("native"));

        let withdraw = vec![
            LedgerEntryChange::State(pool_entry(1_500, 3_000)),
            LedgerEntryChange::Updated(pool_entry(1_000, 2_000)),
        ];
        let delta = pool_delta_details(&withdraw, &pool_id).expect("withdraw delta");
        assert_eq!(delta["amountA"], json!(-500));
        assert_eq!(delta["amountB"], json!(-1_000));
    }

    /// The two boundaries where an image is missing: a pool created by its
    /// first deposit has no pre-image, and one emptied by its last withdrawal
    /// is `Removed` with no post-image.
    #[test]
    fn pool_delta_handles_pool_creation_and_removal() {
        let pool_id = PoolId(Hash([0x44; 32]));
        let created = vec![LedgerEntryChange::Created(pool_entry(1_000, 2_000))];
        let delta = pool_delta_details(&created, &pool_id).expect("created delta");
        assert_eq!(delta["amountA"], json!(1_000));
        assert_eq!(delta["amountB"], json!(2_000));

        let emptied = vec![
            LedgerEntryChange::State(pool_entry(1_000, 2_000)),
            LedgerEntryChange::Removed(LedgerKey::LiquidityPool(LedgerKeyLiquidityPool {
                liquidity_pool_id: pool_id.clone(),
            })),
        ];
        let delta = pool_delta_details(&emptied, &pool_id).expect("removed delta");
        assert_eq!(delta["amountA"], json!(-1_000));
        assert_eq!(delta["amountB"], json!(-2_000));
    }

    /// An op that touches one pool repeatedly must be measured end to end —
    /// first pre-image to last post-image — not just its final touch. No
    /// current caller emits this shape (deposits and withdrawals touch a pool
    /// once), but a path payment would: CAP-38 interleaved matching writes a
    /// State/Updated pair per fill, and reading the LAST `State` would report
    /// only the last fill.
    #[test]
    fn pool_delta_spans_every_touch_of_one_op() {
        let pool_id = PoolId(Hash([0x44; 32]));
        let two_fills = vec![
            LedgerEntryChange::State(pool_entry(1_000, 2_000)),
            LedgerEntryChange::Updated(pool_entry(1_400, 1_500)),
            LedgerEntryChange::State(pool_entry(1_400, 1_500)),
            LedgerEntryChange::Updated(pool_entry(1_900, 900)),
        ];
        let delta = pool_delta_details(&two_fills, &pool_id).expect("delta");
        // 1_900 - 1_000 and 900 - 2_000, not the second fill's 500 / -600.
        assert_eq!(delta["amountA"], json!(900));
        assert_eq!(delta["amountB"], json!(-1_100));
    }

    /// Another pool's entry in the same op must not be read as this pool's.
    #[test]
    fn pool_delta_ignores_a_different_pool() {
        let changes = vec![
            LedgerEntryChange::State(pool_entry(1_000, 2_000)),
            LedgerEntryChange::Updated(pool_entry(1_500, 3_000)),
        ];
        assert!(pool_delta_details(&changes, &PoolId(Hash([0x99; 32]))).is_none());
    }

    #[test]
    fn extract_payment_operation() {
        let op = Operation {
            source_account: None,
            body: OperationBody::Payment(PaymentOp {
                destination: MuxedAccount::Ed25519(Uint256([0xBB; 32])),
                asset: Asset::Native,
                amount: 10_000_000,
            }),
        };
        let inner_ops = vec![op];
        let tx = build_v1_tx(inner_ops);
        let inner = InnerTxRef::V1(&tx);
        let result = extract_operations(&inner, None, None, "abcd1234", 100, 0);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].op_type, OperationType::Payment);
        assert_eq!(result[0].transaction_hash, "abcd1234");
        assert_eq!(result[0].operation_index, 1);
        assert!(result[0].source_account.is_none());
        assert_eq!(result[0].details["asset"], "native");
        assert_eq!(result[0].details["amount"], 10_000_000);
    }

    #[test]
    fn extract_create_account_operation() {
        let op = Operation {
            source_account: Some(MuxedAccount::Ed25519(Uint256([0xAA; 32]))),
            body: OperationBody::CreateAccount(CreateAccountOp {
                destination: AccountId(PublicKey::PublicKeyTypeEd25519(Uint256([0xCC; 32]))),
                starting_balance: 100_000_000,
            }),
        };
        let tx = build_v1_tx(vec![op]);
        let inner = InnerTxRef::V1(&tx);
        let result = extract_operations(&inner, None, None, "abcd1234", 100, 0);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].op_type, OperationType::CreateAccount);
        assert!(result[0].source_account.is_some());
        assert_eq!(result[0].details["startingBalance"], 100_000_000);
    }

    #[test]
    fn extract_invoke_host_function_with_args() {
        let contract_addr = ScAddress::Contract(ContractId(Hash([0xDD; 32])));
        let func_name = ScSymbol::try_from("transfer".as_bytes().to_vec()).unwrap();
        let args = vec![ScVal::U64(42), ScVal::Bool(true)].try_into().unwrap();

        let op = Operation {
            source_account: None,
            body: OperationBody::InvokeHostFunction(InvokeHostFunctionOp {
                host_function: HostFunction::InvokeContract(InvokeContractArgs {
                    contract_address: contract_addr,
                    function_name: func_name,
                    args,
                }),
                auth: VecM::default(),
            }),
        };

        let return_val = ScVal::I128(Int128Parts { hi: 0, lo: 999 });
        let soroban_meta = SorobanTransactionMeta {
            ext: SorobanTransactionMetaExt::V0,
            events: VecM::default(),
            return_value: return_val,
            diagnostic_events: VecM::default(),
        };
        let tx_meta = TransactionMeta::V3(TransactionMetaV3 {
            ext: ExtensionPoint::V0,
            tx_changes_before: LedgerEntryChanges::default(),
            operations: VecM::default(),
            tx_changes_after: LedgerEntryChanges::default(),
            soroban_meta: Some(soroban_meta),
        });

        let tx = build_v1_tx(vec![op]);
        let inner = InnerTxRef::V1(&tx);
        let result = extract_operations(&inner, Some(&tx_meta), None, "abcd1234", 100, 0);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].op_type, OperationType::InvokeHostFunction);
        let details = &result[0].details;
        assert_eq!(details["hostFunctionType"], "invokeContract");
        assert_eq!(details["functionName"], "transfer");
        assert!(!details["contractId"].as_str().unwrap().is_empty());

        // Check function args are ScVal-decoded
        let args = details["functionArgs"].as_array().unwrap();
        assert_eq!(args.len(), 2);
        assert_eq!(args[0]["type"], "u64");
        assert_eq!(args[0]["value"], 42);
        assert_eq!(args[1]["type"], "bool");
        assert_eq!(args[1]["value"], true);

        // Check return value is ScVal-decoded
        let ret = &details["returnValue"];
        assert_eq!(ret["type"], "i128");
        assert_eq!(ret["value"], "999");
    }

    #[test]
    fn extract_invoke_upload_wasm() {
        let wasm_bytes = BytesM::try_from(vec![0u8; 256]).unwrap();
        let op = Operation {
            source_account: None,
            body: OperationBody::InvokeHostFunction(InvokeHostFunctionOp {
                host_function: HostFunction::UploadContractWasm(wasm_bytes),
                auth: VecM::default(),
            }),
        };
        let tx = build_v1_tx(vec![op]);
        let inner = InnerTxRef::V1(&tx);
        let result = extract_operations(&inner, None, None, "abcd1234", 100, 0);

        assert_eq!(result[0].details["hostFunctionType"], "uploadContractWasm");
        assert_eq!(result[0].details["wasmLength"], 256);
    }

    #[test]
    fn extract_multiple_operations_preserves_order() {
        let ops = vec![
            Operation {
                source_account: None,
                body: OperationBody::Inflation,
            },
            Operation {
                source_account: None,
                body: OperationBody::BumpSequence(BumpSequenceOp {
                    bump_to: SequenceNumber(42),
                }),
            },
            Operation {
                source_account: None,
                body: OperationBody::EndSponsoringFutureReserves,
            },
        ];
        let tx = build_v1_tx(ops);
        let inner = InnerTxRef::V1(&tx);
        let result = extract_operations(&inner, None, None, "abcd1234", 100, 0);

        assert_eq!(result.len(), 3);
        assert_eq!(result[0].operation_index, 1);
        assert_eq!(result[0].op_type, OperationType::Inflation);
        assert_eq!(result[1].operation_index, 2);
        assert_eq!(result[1].op_type, OperationType::BumpSequence);
        assert_eq!(result[1].details["bumpTo"], 42);
        assert_eq!(result[2].operation_index, 3);
        assert_eq!(
            result[2].op_type,
            OperationType::EndSponsoringFutureReserves
        );
    }

    #[test]
    fn manage_data_with_value() {
        let op = Operation {
            source_account: None,
            body: OperationBody::ManageData(ManageDataOp {
                data_name: String64::try_from("mykey".as_bytes().to_vec()).unwrap(),
                data_value: Some(DataValue::try_from(vec![0xDE, 0xAD]).unwrap()),
            }),
        };
        let tx = build_v1_tx(vec![op]);
        let inner = InnerTxRef::V1(&tx);
        let result = extract_operations(&inner, None, None, "abcd1234", 100, 0);

        assert_eq!(result[0].op_type, OperationType::ManageData);
        assert_eq!(result[0].details["name"], "mykey");
        // base64 of [0xDE, 0xAD] = "3q0="
        assert_eq!(result[0].details["value"], "3q0=");
    }

    #[test]
    fn manage_sell_offer_details() {
        let op = Operation {
            source_account: None,
            body: OperationBody::ManageSellOffer(ManageSellOfferOp {
                selling: Asset::Native,
                buying: Asset::Native,
                amount: 500,
                price: Price { n: 1, d: 2 },
                offer_id: 123,
            }),
        };
        let tx = build_v1_tx(vec![op]);
        let inner = InnerTxRef::V1(&tx);
        let result = extract_operations(&inner, None, None, "abcd1234", 100, 0);

        assert_eq!(result[0].op_type, OperationType::ManageSellOffer);
        assert_eq!(result[0].details["amount"], 500);
        assert_eq!(result[0].details["price"]["n"], 1);
        assert_eq!(result[0].details["price"]["d"], 2);
        assert_eq!(result[0].details["offerId"], 123);
    }

    #[test]
    fn set_options_partial_fields() {
        let op = Operation {
            source_account: None,
            body: OperationBody::SetOptions(SetOptionsOp {
                inflation_dest: None,
                clear_flags: Some(1),
                set_flags: None,
                master_weight: Some(10),
                low_threshold: None,
                med_threshold: None,
                high_threshold: None,
                home_domain: None,
                signer: None,
            }),
        };
        let tx = build_v1_tx(vec![op]);
        let inner = InnerTxRef::V1(&tx);
        let result = extract_operations(&inner, None, None, "abcd1234", 100, 0);

        assert_eq!(result[0].op_type, OperationType::SetOptions);
        assert_eq!(result[0].details["clearFlags"], 1);
        assert_eq!(result[0].details["masterWeight"], 10);
        // Fields not set should not be present
        assert!(result[0].details.get("inflationDest").is_none());
        assert!(result[0].details.get("setFlags").is_none());
    }

    #[test]
    fn extend_footprint_ttl_details() {
        let op = Operation {
            source_account: None,
            body: OperationBody::ExtendFootprintTtl(ExtendFootprintTtlOp {
                ext: ExtensionPoint::V0,
                extend_to: 1000,
            }),
        };
        let tx = build_v1_tx(vec![op]);
        let inner = InnerTxRef::V1(&tx);
        let result = extract_operations(&inner, None, None, "abcd1234", 100, 0);

        assert_eq!(result[0].op_type, OperationType::ExtendFootprintTtl);
        assert_eq!(result[0].details["extendTo"], 1000);
    }

    #[test]
    fn restore_footprint_details() {
        let op = Operation {
            source_account: None,
            body: OperationBody::RestoreFootprint(RestoreFootprintOp {
                ext: ExtensionPoint::V0,
            }),
        };
        let tx = build_v1_tx(vec![op]);
        let inner = InnerTxRef::V1(&tx);
        let result = extract_operations(&inner, None, None, "abcd1234", 100, 0);

        assert_eq!(result[0].op_type, OperationType::RestoreFootprint);
        assert_eq!(result[0].details, json!({}));
    }

    #[test]
    fn invoke_host_function_without_meta_has_null_return() {
        let contract_addr = ScAddress::Contract(ContractId(Hash([0xDD; 32])));
        let func_name = ScSymbol::try_from("hello".as_bytes().to_vec()).unwrap();
        let op = Operation {
            source_account: None,
            body: OperationBody::InvokeHostFunction(InvokeHostFunctionOp {
                host_function: HostFunction::InvokeContract(InvokeContractArgs {
                    contract_address: contract_addr,
                    function_name: func_name,
                    args: VecM::default(),
                }),
                auth: VecM::default(),
            }),
        };
        let tx = build_v1_tx(vec![op]);
        let inner = InnerTxRef::V1(&tx);
        let result = extract_operations(&inner, None, None, "abcd1234", 100, 0);

        assert_eq!(result[0].details["hostFunctionType"], "invokeContract");
        assert!(result[0].details["returnValue"].is_null());
    }

    // --- path-payment pool claims (task 0261) ---

    #[test]
    fn path_payment_strict_send_single_pool() {
        let op = build_path_payment_send_op();
        let tx = build_v1_tx(vec![op]);
        let inner = InnerTxRef::V1(&tx);
        let results = vec![path_payment_send_result(vec![
            order_book_atom(),
            lp_atom([0x11; 32], 500, 200),
        ])];

        let result = extract_operations(&inner, None, Some(&results), "abcd1234", 100, 0);

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].op_type, OperationType::PathPaymentStrictSend);
        let details = &result[0].details;
        let pool_ids = details["poolIds"].as_array().unwrap();
        assert_eq!(pool_ids.len(), 1);
        assert_eq!(pool_ids[0], hex::encode([0x11; 32]));
        let atoms = details["claimedAtoms"].as_array().unwrap();
        assert_eq!(atoms.len(), 1);
        assert_eq!(atoms[0]["poolId"], hex::encode([0x11; 32]));
        assert_eq!(atoms[0]["amountSold"], 500);
        assert_eq!(atoms[0]["amountBought"], 200);
        assert_eq!(atoms[0]["assetSold"], "native");
        // both legs native here → amountA = amountSold side.
        assert_eq!(atoms[0]["amountA"], 500);
    }

    #[test]
    fn amount_a_picks_canonical_asset_a_side() {
        // native < credit (XDR Asset order: type 0 < type 1), so asset A is
        // native regardless of which leg of the atom it is.
        let credit = Asset::CreditAlphanum4(AlphaNum4 {
            asset_code: AssetCode4(*b"USDC"),
            issuer: AccountId(PublicKey::PublicKeyTypeEd25519(Uint256([0x09; 32]))),
        });
        // sold = native (A), bought = credit (B) → amountA = amountSold.
        let a1 = ClaimAtom::LiquidityPool(ClaimLiquidityAtom {
            liquidity_pool_id: PoolId(Hash([0x11; 32])),
            asset_sold: Asset::Native,
            amount_sold: 700,
            asset_bought: credit.clone(),
            amount_bought: 300,
        });
        // sold = credit (B), bought = native (A) → amountA = amountBought.
        let a2 = ClaimAtom::LiquidityPool(ClaimLiquidityAtom {
            liquidity_pool_id: PoolId(Hash([0x22; 32])),
            asset_sold: credit,
            amount_sold: 250,
            asset_bought: Asset::Native,
            amount_bought: 900,
        });
        let op = build_path_payment_send_op();
        let tx = build_v1_tx(vec![op]);
        let inner = InnerTxRef::V1(&tx);
        let results = vec![path_payment_send_result(vec![a1, a2])];

        let result = extract_operations(&inner, None, Some(&results), "abcd1234", 100, 0);
        let atoms = result[0].details["claimedAtoms"].as_array().unwrap();
        assert_eq!(atoms[0]["amountA"], 700, "native is A, on the sold side");
        assert_eq!(atoms[1]["amountA"], 900, "native is A, on the bought side");
    }

    #[test]
    fn path_payment_strict_receive_multi_hop_dedups_pool_ids() {
        let op = Operation {
            source_account: None,
            body: OperationBody::PathPaymentStrictReceive(PathPaymentStrictReceiveOp {
                send_asset: Asset::Native,
                send_max: 1_000,
                destination: MuxedAccount::Ed25519(Uint256([0xBB; 32])),
                dest_asset: Asset::Native,
                dest_amount: 900,
                path: VecM::default(),
            }),
        };
        let tx = build_v1_tx(vec![op]);
        let inner = InnerTxRef::V1(&tx);
        // Two distinct pools + a repeat crossing of the first pool.
        let results = vec![path_payment_receive_result(vec![
            lp_atom([0x11; 32], 500, 200),
            lp_atom([0x22; 32], 200, 150),
            lp_atom([0x11; 32], 100, 40),
        ])];

        let result = extract_operations(&inner, None, Some(&results), "abcd1234", 100, 0);

        assert_eq!(result[0].op_type, OperationType::PathPaymentStrictReceive);
        let details = &result[0].details;
        let pool_ids = details["poolIds"].as_array().unwrap();
        assert_eq!(pool_ids.len(), 2, "pool ids deduped, order preserved");
        assert_eq!(pool_ids[0], hex::encode([0x11; 32]));
        assert_eq!(pool_ids[1], hex::encode([0x22; 32]));
        // claimedAtoms keeps every fill (amounts feed gross_volume_a).
        assert_eq!(details["claimedAtoms"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn failed_path_payment_has_no_pool_claims() {
        let op = build_path_payment_send_op();
        let tx = build_v1_tx(vec![op]);
        let inner = InnerTxRef::V1(&tx);
        let results = vec![OperationResult::OpInner(
            OperationResultTr::PathPaymentStrictSend(PathPaymentStrictSendResult::Underfunded),
        )];

        let result = extract_operations(&inner, None, Some(&results), "abcd1234", 100, 0);

        let details = &result[0].details;
        assert!(details.get("poolIds").is_none());
        assert!(details.get("claimedAtoms").is_none());
    }

    #[test]
    fn order_book_only_path_payment_has_no_pool_claims() {
        let op = build_path_payment_send_op();
        let tx = build_v1_tx(vec![op]);
        let inner = InnerTxRef::V1(&tx);
        let results = vec![path_payment_send_result(vec![order_book_atom()])];

        let result = extract_operations(&inner, None, Some(&results), "abcd1234", 100, 0);

        let details = &result[0].details;
        assert!(details.get("poolIds").is_none());
        assert!(details.get("claimedAtoms").is_none());
    }

    #[test]
    fn plain_payment_ignores_op_results() {
        let op = Operation {
            source_account: None,
            body: OperationBody::Payment(PaymentOp {
                destination: MuxedAccount::Ed25519(Uint256([0xBB; 32])),
                asset: Asset::Native,
                amount: 10,
            }),
        };
        let tx = build_v1_tx(vec![op]);
        let inner = InnerTxRef::V1(&tx);
        let results = vec![OperationResult::OpInner(OperationResultTr::Payment(
            PaymentResult::Success,
        ))];

        let result = extract_operations(&inner, None, Some(&results), "abcd1234", 100, 0);

        assert!(result[0].details.get("poolIds").is_none());
        assert!(result[0].details.get("claimedAtoms").is_none());
    }

    #[test]
    fn tx_op_results_success_only_and_unwraps_fee_bump() {
        let ops: VecM<OperationResult> = vec![OperationResult::OpInner(
            OperationResultTr::Payment(PaymentResult::Success),
        )]
        .try_into()
        .unwrap();

        let plain = build_tx_result(TransactionResultResult::TxSuccess(ops.clone()));
        assert_eq!(tx_op_results(&plain).unwrap().len(), 1);

        // TxFailed rolls every op back; op-level Success results in it must NOT
        // yield pool data (task 0261 — phantom-crossing guard).
        let failed = build_tx_result(TransactionResultResult::TxFailed(ops.clone()));
        assert!(tx_op_results(&failed).is_none());

        let inner_ok = InnerTransactionResultPair {
            transaction_hash: Hash([0xEE; 32]),
            result: InnerTransactionResult {
                fee_charged: 100,
                result: InnerTransactionResultResult::TxSuccess(ops.clone()),
                ext: InnerTransactionResultExt::V0,
            },
        };
        let fee_bump = build_tx_result(TransactionResultResult::TxFeeBumpInnerSuccess(inner_ok));
        assert_eq!(
            tx_op_results(&fee_bump).unwrap().len(),
            1,
            "fee-bump nesting unwrapped"
        );

        // Fee-bump wrapping a FAILED inner tx → no op results.
        let inner_fail = InnerTransactionResultPair {
            transaction_hash: Hash([0xEE; 32]),
            result: InnerTransactionResult {
                fee_charged: 100,
                result: InnerTransactionResultResult::TxFailed(ops),
                ext: InnerTransactionResultExt::V0,
            },
        };
        let fee_bump_fail =
            build_tx_result(TransactionResultResult::TxFeeBumpInnerFailed(inner_fail));
        assert!(tx_op_results(&fee_bump_fail).is_none());

        let validation_failed = build_tx_result(TransactionResultResult::TxBadSeq);
        assert!(tx_op_results(&validation_failed).is_none());
    }

    #[test]
    fn tx_op_results_any_covers_failed_arms() {
        // The 0352 fixture shape (7af6d0ed…): TxFailed still carries the
        // per-op array — success op, LowReserve, op-level OpNoAccount.
        let ops: VecM<OperationResult> = vec![
            OperationResult::OpInner(OperationResultTr::BeginSponsoringFutureReserves(
                BeginSponsoringFutureReservesResult::Success,
            )),
            OperationResult::OpInner(OperationResultTr::CreateAccount(
                CreateAccountResult::LowReserve,
            )),
            OperationResult::OpNoAccount,
        ]
        .try_into()
        .unwrap();

        let failed = build_tx_result(TransactionResultResult::TxFailed(ops.clone()));
        let results = tx_op_results_any(&failed).expect("failed arm carries op results");
        assert_eq!(
            results.iter().map(op_result_code).collect::<Vec<_>>(),
            vec!["Success", "LowReserve", "OpNoAccount"]
        );

        let inner_fail = InnerTransactionResultPair {
            transaction_hash: Hash([0xEE; 32]),
            result: InnerTransactionResult {
                fee_charged: 100,
                result: InnerTransactionResultResult::TxFailed(ops),
                ext: InnerTransactionResultExt::V0,
            },
        };
        let fee_bump_fail =
            build_tx_result(TransactionResultResult::TxFeeBumpInnerFailed(inner_fail));
        assert_eq!(
            tx_op_results_any(&fee_bump_fail).map(<[OperationResult]>::len),
            Some(3),
            "fee-bump failed inner unwrapped"
        );

        // Validation-level failure: no op was attempted, no array exists.
        let validation_failed = build_tx_result(TransactionResultResult::TxBadSeq);
        assert!(tx_op_results_any(&validation_failed).is_none());
    }

    #[test]
    fn failed_tx_drops_pool_claims_for_op_level_success() {
        // op0 = path payment that crossed pool P at the op level, but the whole
        // tx failed (a later op failed). tx_op_results returns None for the
        // failed tx → no poolIds, so rolled-back crossings stay out of the DB
        // and out of gross_volume_a.
        let op = build_path_payment_send_op();
        let tx = build_v1_tx(vec![op]);
        let inner = InnerTxRef::V1(&tx);
        let ops: VecM<OperationResult> = vec![path_payment_send_result(vec![lp_atom(
            [0x11; 32], 500, 200,
        )])]
        .try_into()
        .unwrap();
        let tx_result = build_tx_result(TransactionResultResult::TxFailed(ops));

        let op_results = tx_op_results(&tx_result);
        assert!(op_results.is_none(), "failed tx exposes no op results");

        let result = extract_operations(&inner, None, op_results, "abcd1234", 100, 0);
        assert!(result[0].details.get("poolIds").is_none());
        assert!(result[0].details.get("claimedAtoms").is_none());
    }

    #[test]
    fn manage_buy_offer_crossing_pool_tags_pool_ids() {
        // An offer that fills against an AMM carries LP claim atoms in its
        // result (ManageOfferSuccessResult.offers_claimed) — the unified
        // extractor must surface them too (task 0261 / 0266 single-run scope).
        let op = Operation {
            source_account: None,
            body: OperationBody::ManageBuyOffer(ManageBuyOfferOp {
                selling: Asset::Native,
                buying: Asset::Native,
                buy_amount: 500,
                price: Price { n: 1, d: 2 },
                offer_id: 0,
            }),
        };
        let tx = build_v1_tx(vec![op]);
        let inner = InnerTxRef::V1(&tx);
        let results = vec![manage_buy_offer_result(vec![
            order_book_atom(),
            lp_atom([0x33; 32], 700, 350),
        ])];

        let result = extract_operations(&inner, None, Some(&results), "abcd1234", 100, 0);

        assert_eq!(result[0].op_type, OperationType::ManageBuyOffer);
        let details = &result[0].details;
        let pool_ids = details["poolIds"].as_array().unwrap();
        assert_eq!(pool_ids.len(), 1);
        assert_eq!(pool_ids[0], hex::encode([0x33; 32]));
        assert_eq!(details["claimedAtoms"].as_array().unwrap().len(), 1);
        // Plain (non-LP) offer fields are preserved alongside the claims.
        assert_eq!(details["offerId"], 0);
    }

    // --- test helpers ---

    fn build_path_payment_send_op() -> Operation {
        Operation {
            source_account: None,
            body: OperationBody::PathPaymentStrictSend(PathPaymentStrictSendOp {
                send_asset: Asset::Native,
                send_amount: 1_000,
                destination: MuxedAccount::Ed25519(Uint256([0xBB; 32])),
                dest_asset: Asset::Native,
                dest_min: 900,
                path: VecM::default(),
            }),
        }
    }

    fn lp_atom(pool_id: [u8; 32], amount_sold: i64, amount_bought: i64) -> ClaimAtom {
        ClaimAtom::LiquidityPool(ClaimLiquidityAtom {
            liquidity_pool_id: PoolId(Hash(pool_id)),
            asset_sold: Asset::Native,
            amount_sold,
            asset_bought: Asset::Native,
            amount_bought,
        })
    }

    fn order_book_atom() -> ClaimAtom {
        ClaimAtom::OrderBook(ClaimOfferAtom {
            seller_id: AccountId(PublicKey::PublicKeyTypeEd25519(Uint256([0xCC; 32]))),
            offer_id: 7,
            asset_sold: Asset::Native,
            amount_sold: 10,
            asset_bought: Asset::Native,
            amount_bought: 10,
        })
    }

    fn simple_payment_result() -> SimplePaymentResult {
        SimplePaymentResult {
            destination: AccountId(PublicKey::PublicKeyTypeEd25519(Uint256([0xBB; 32]))),
            asset: Asset::Native,
            amount: 900,
        }
    }

    fn path_payment_send_result(offers: Vec<ClaimAtom>) -> OperationResult {
        OperationResult::OpInner(OperationResultTr::PathPaymentStrictSend(
            PathPaymentStrictSendResult::Success(PathPaymentStrictSendResultSuccess {
                offers: offers.try_into().unwrap(),
                last: simple_payment_result(),
            }),
        ))
    }

    fn path_payment_receive_result(offers: Vec<ClaimAtom>) -> OperationResult {
        OperationResult::OpInner(OperationResultTr::PathPaymentStrictReceive(
            PathPaymentStrictReceiveResult::Success(PathPaymentStrictReceiveResultSuccess {
                offers: offers.try_into().unwrap(),
                last: simple_payment_result(),
            }),
        ))
    }

    fn manage_buy_offer_result(claimed: Vec<ClaimAtom>) -> OperationResult {
        OperationResult::OpInner(OperationResultTr::ManageBuyOffer(
            ManageBuyOfferResult::Success(ManageOfferSuccessResult {
                offers_claimed: claimed.try_into().unwrap(),
                offer: ManageOfferSuccessResultOffer::Deleted,
            }),
        ))
    }

    fn build_tx_result(result: TransactionResultResult) -> TransactionResult {
        TransactionResult {
            fee_charged: 100,
            result,
            ext: TransactionResultExt::V0,
        }
    }

    fn build_v1_tx(operations: Vec<Operation>) -> Transaction {
        Transaction {
            source_account: MuxedAccount::Ed25519(Uint256([0xAA; 32])),
            fee: 100,
            seq_num: SequenceNumber(1),
            cond: Preconditions::None,
            memo: Memo::None,
            operations: operations.try_into().unwrap(),
            ext: TransactionExt::V0,
        }
    }

    #[test]
    fn create_claimable_balance_lists_claimants() {
        // Task 0460 #16: `claimants` used to be a bare count — now it IS the
        // vec; addresses and the (recursive) predicates must survive.
        let claimant = |byte: u8, predicate: ClaimPredicate| {
            Claimant::ClaimantTypeV0(ClaimantV0 {
                destination: AccountId(PublicKey::PublicKeyTypeEd25519(Uint256([byte; 32]))),
                predicate,
            })
        };
        let op = Operation {
            source_account: None,
            body: OperationBody::CreateClaimableBalance(CreateClaimableBalanceOp {
                asset: Asset::Native,
                amount: 5_000,
                claimants: vec![
                    claimant(0xBB, ClaimPredicate::Unconditional),
                    claimant(
                        0xCC,
                        ClaimPredicate::Not(Some(Box::new(ClaimPredicate::BeforeAbsoluteTime(
                            1_700_000_000,
                        )))),
                    ),
                ]
                .try_into()
                .unwrap(),
            }),
        };
        let tx = build_v1_tx(vec![op]);
        let inner = InnerTxRef::V1(&tx);

        let result = extract_operations(&inner, None, None, "abcd1234", 100, 0);
        let details = &result[0].details;

        let list = details["claimants"].as_array().unwrap();
        assert_eq!(list.len(), 2);
        let dest = list[0]["destination"].as_str().unwrap();
        assert!(
            dest.starts_with('G') && dest.len() == 56,
            "destination must be a G-strkey, got {dest}"
        );
        assert_eq!(list[0]["predicate"]["type"], "unconditional");
        assert_eq!(list[1]["predicate"]["type"], "not");
        assert_eq!(
            list[1]["predicate"]["predicate"]["type"],
            "beforeAbsoluteTime"
        );
        assert_eq!(
            list[1]["predicate"]["predicate"]["timePoint"],
            1_700_000_000_i64
        );
    }
}
