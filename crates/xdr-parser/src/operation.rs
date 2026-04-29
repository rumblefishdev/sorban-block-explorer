//! Operation extraction from transaction envelopes.
//!
//! Extracts per-operation structured data with type-specific JSONB details.
//! INVOKE_HOST_FUNCTION operations get enriched extraction: contractId,
//! functionName, functionArgs (ScVal decoded), and returnValue.

use crate::envelope::{InnerTxRef, muxed_to_g_strkey};
use crate::scval::scval_to_typed_json;
use crate::types::ExtractedOperation;
use domain::OperationType;
use serde_json::{Value, json};
use stellar_xdr::curr::*;

/// Extract all operations from a transaction envelope, with optional return
/// value from the transaction meta (for INVOKE_HOST_FUNCTION).
///
/// `tx_meta` is needed to extract the Soroban return value. Pass the
/// `TransactionMeta` from the processing result.
pub fn extract_operations(
    envelope: &InnerTxRef<'_>,
    tx_meta: Option<&TransactionMeta>,
    transaction_hash: &str,
    ledger_sequence: u32,
    tx_index: usize,
) -> Vec<ExtractedOperation> {
    let ops = match envelope {
        InnerTxRef::V0(tx) => tx.operations.as_slice(),
        InnerTxRef::V1(tx) => tx.operations.as_slice(),
    };

    let return_value = tx_meta.and_then(soroban_return_value);

    ops.iter()
        .enumerate()
        .map(|(i, op)| {
            // operation_index is 1-based to match Stellar ecosystem convention
            // (Horizon paging_token encodes op_app_order in low 12 bits, also
            // 1-based). Surfaces as user-facing `application_order` in
            // `XdrOperationDto`. See task 0172 / ADR 0028.
            let op_index = i + 1;
            let source_account = op.source_account.as_ref().map(muxed_to_g_strkey);
            let (op_type, details) = extract_op_details(
                &op.body,
                return_value.as_ref(),
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
            }
        })
        .collect()
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

/// Extract operation type discriminator and details JSON for a single
/// operation. Matches the XDR body variant; the resulting `OperationType`
/// casts to SMALLINT via `#[repr(i16)]` with zero lookup cost.
fn extract_op_details(
    body: &OperationBody,
    return_value: Option<&ScVal>,
    _ledger_sequence: u32,
    _tx_index: usize,
    _op_index: usize,
) -> (OperationType, Value) {
    match body {
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
                "claimants": op.claimants.len(),
            }),
        ),
        OperationBody::ClaimClaimableBalance(op) => (
            OperationType::ClaimClaimableBalance,
            json!({
                "balanceId": format_claimable_balance_id(&op.balance_id),
            }),
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
            json!({
                "balanceId": format_claimable_balance_id(&op.balance_id),
            }),
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
        OperationBody::LiquidityPoolDeposit(op) => (
            OperationType::LiquidityPoolDeposit,
            json!({
                "liquidityPoolId": hex::encode(op.liquidity_pool_id.0.as_slice()),
                "maxAmountA": op.max_amount_a,
                "maxAmountB": op.max_amount_b,
                "minPrice": format_price(&op.min_price),
                "maxPrice": format_price(&op.max_price),
            }),
        ),
        OperationBody::LiquidityPoolWithdraw(op) => (
            OperationType::LiquidityPoolWithdraw,
            json!({
                "liquidityPoolId": hex::encode(op.liquidity_pool_id.0.as_slice()),
                "amount": op.amount,
                "minAmountA": op.min_amount_a,
                "minAmountB": op.min_amount_b,
            }),
        ),
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
    }
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
            let code = std::str::from_utf8(a.asset_code.as_slice())
                .unwrap_or("<invalid>")
                .trim_end_matches('\0');
            json!(format!("{}:{}", code, a.issuer.0.to_string()))
        }
        Asset::CreditAlphanum12(a) => {
            let code = std::str::from_utf8(a.asset_code.as_slice())
                .unwrap_or("<invalid>")
                .trim_end_matches('\0');
            json!(format!("{}:{}", code, a.issuer.0.to_string()))
        }
    }
}

fn format_change_trust_asset(asset: &ChangeTrustAsset) -> Value {
    match asset {
        ChangeTrustAsset::Native => json!("native"),
        ChangeTrustAsset::CreditAlphanum4(a) => {
            let code = std::str::from_utf8(a.asset_code.as_slice())
                .unwrap_or("<invalid>")
                .trim_end_matches('\0');
            json!(format!("{}:{}", code, a.issuer.0.to_string()))
        }
        ChangeTrustAsset::CreditAlphanum12(a) => {
            let code = std::str::from_utf8(a.asset_code.as_slice())
                .unwrap_or("<invalid>")
                .trim_end_matches('\0');
            json!(format!("{}:{}", code, a.issuer.0.to_string()))
        }
        ChangeTrustAsset::PoolShare(params) => {
            json!({ "type": "liquidityPool", "params": params.name() })
        }
    }
}

fn format_asset_code(code: &AssetCode) -> Value {
    let s = match code {
        AssetCode::CreditAlphanum4(c) => std::str::from_utf8(c.as_slice())
            .unwrap_or("<invalid>")
            .trim_end_matches('\0')
            .to_string(),
        AssetCode::CreditAlphanum12(c) => std::str::from_utf8(c.as_slice())
            .unwrap_or("<invalid>")
            .trim_end_matches('\0')
            .to_string(),
    };
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

fn format_contract_executable(exec: &ContractExecutable) -> Value {
    match exec {
        ContractExecutable::Wasm(hash) => json!({ "type": "wasm", "hash": hex::encode(hash.0) }),
        ContractExecutable::StellarAsset => json!({ "type": "stellar_asset" }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let result = extract_operations(&inner, None, "abcd1234", 100, 0);

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
        let result = extract_operations(&inner, None, "abcd1234", 100, 0);

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
        let result = extract_operations(&inner, Some(&tx_meta), "abcd1234", 100, 0);

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
        let result = extract_operations(&inner, None, "abcd1234", 100, 0);

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
        let result = extract_operations(&inner, None, "abcd1234", 100, 0);

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
        let result = extract_operations(&inner, None, "abcd1234", 100, 0);

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
        let result = extract_operations(&inner, None, "abcd1234", 100, 0);

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
        let result = extract_operations(&inner, None, "abcd1234", 100, 0);

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
        let result = extract_operations(&inner, None, "abcd1234", 100, 0);

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
        let result = extract_operations(&inner, None, "abcd1234", 100, 0);

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
        let result = extract_operations(&inner, None, "abcd1234", 100, 0);

        assert_eq!(result[0].details["hostFunctionType"], "invokeContract");
        assert!(result[0].details["returnValue"].is_null());
    }

    // --- test helpers ---

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
}
