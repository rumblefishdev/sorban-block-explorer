//! Operation extraction from transaction envelopes.
//!
//! Extracts per-operation structured data with type-specific JSONB details.
//! INVOKE_HOST_FUNCTION operations get enriched extraction: contractId,
//! functionName, functionArgs (ScVal decoded), and returnValue.

use crate::envelope::InnerTxRef;
use crate::scval::scval_to_typed_json;
use crate::types::ExtractedOperation;
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
            let source_account = op.source_account.as_ref().map(|a| a.to_string());
            let (op_type, details) = extract_op_details(
                &op.body,
                return_value.as_ref(),
                ledger_sequence,
                tx_index,
                i,
            );
            ExtractedOperation {
                transaction_hash: transaction_hash.to_string(),
                operation_index: u32::try_from(i).expect("operation index does not fit into u32"),
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

/// Extract operation type string and details JSON for a single operation.
fn extract_op_details(
    body: &OperationBody,
    return_value: Option<&ScVal>,
    _ledger_sequence: u32,
    _tx_index: usize,
    _op_index: usize,
) -> (String, Value) {
    match body {
        OperationBody::CreateAccount(op) => (
            "CREATE_ACCOUNT".into(),
            json!({
                "destination": op.destination.0.to_string(),
                "startingBalance": op.starting_balance,
            }),
        ),
        OperationBody::Payment(op) => (
            "PAYMENT".into(),
            json!({
                "destination": op.destination.to_string(),
                "asset": format_asset(&op.asset),
                "amount": op.amount,
            }),
        ),
        OperationBody::PathPaymentStrictReceive(op) => (
            "PATH_PAYMENT_STRICT_RECEIVE".into(),
            json!({
                "sendAsset": format_asset(&op.send_asset),
                "sendMax": op.send_max,
                "destination": op.destination.to_string(),
                "destAsset": format_asset(&op.dest_asset),
                "destAmount": op.dest_amount,
                "path": op.path.iter().map(format_asset).collect::<Vec<_>>(),
            }),
        ),
        OperationBody::PathPaymentStrictSend(op) => (
            "PATH_PAYMENT_STRICT_SEND".into(),
            json!({
                "sendAsset": format_asset(&op.send_asset),
                "sendAmount": op.send_amount,
                "destination": op.destination.to_string(),
                "destAsset": format_asset(&op.dest_asset),
                "destMin": op.dest_min,
                "path": op.path.iter().map(format_asset).collect::<Vec<_>>(),
            }),
        ),
        OperationBody::ManageSellOffer(op) => (
            "MANAGE_SELL_OFFER".into(),
            json!({
                "selling": format_asset(&op.selling),
                "buying": format_asset(&op.buying),
                "amount": op.amount,
                "price": format_price(&op.price),
                "offerId": op.offer_id,
            }),
        ),
        OperationBody::ManageBuyOffer(op) => (
            "MANAGE_BUY_OFFER".into(),
            json!({
                "selling": format_asset(&op.selling),
                "buying": format_asset(&op.buying),
                "buyAmount": op.buy_amount,
                "price": format_price(&op.price),
                "offerId": op.offer_id,
            }),
        ),
        OperationBody::CreatePassiveSellOffer(op) => (
            "CREATE_PASSIVE_SELL_OFFER".into(),
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
            ("SET_OPTIONS".into(), Value::Object(details))
        }
        OperationBody::ChangeTrust(op) => (
            "CHANGE_TRUST".into(),
            json!({
                "asset": format_change_trust_asset(&op.line),
                "limit": op.limit,
            }),
        ),
        OperationBody::AllowTrust(op) => (
            "ALLOW_TRUST".into(),
            json!({
                "trustor": op.trustor.0.to_string(),
                "asset": format_asset_code(&op.asset),
                "authorize": op.authorize,
            }),
        ),
        OperationBody::AccountMerge(destination) => (
            "ACCOUNT_MERGE".into(),
            json!({
                "destination": destination.to_string(),
            }),
        ),
        OperationBody::Inflation => ("INFLATION".into(), json!({})),
        OperationBody::ManageData(op) => {
            let name = std::str::from_utf8(op.data_name.as_vec()).unwrap_or("<invalid-utf8>");
            let value = op.data_value.as_ref().map(|v| {
                base64::Engine::encode(&base64::engine::general_purpose::STANDARD, v.as_slice())
            });
            (
                "MANAGE_DATA".into(),
                json!({
                    "name": name,
                    "value": value,
                }),
            )
        }
        OperationBody::BumpSequence(op) => (
            "BUMP_SEQUENCE".into(),
            json!({
                "bumpTo": op.bump_to.0,
            }),
        ),
        OperationBody::CreateClaimableBalance(op) => (
            "CREATE_CLAIMABLE_BALANCE".into(),
            json!({
                "asset": format_asset(&op.asset),
                "amount": op.amount,
                "claimants": op.claimants.len(),
            }),
        ),
        OperationBody::ClaimClaimableBalance(op) => (
            "CLAIM_CLAIMABLE_BALANCE".into(),
            json!({
                "balanceId": format_claimable_balance_id(&op.balance_id),
            }),
        ),
        OperationBody::BeginSponsoringFutureReserves(op) => (
            "BEGIN_SPONSORING_FUTURE_RESERVES".into(),
            json!({
                "sponsoredId": op.sponsored_id.0.to_string(),
            }),
        ),
        OperationBody::EndSponsoringFutureReserves => {
            ("END_SPONSORING_FUTURE_RESERVES".into(), json!({}))
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
            ("REVOKE_SPONSORSHIP".into(), details)
        }
        OperationBody::Clawback(op) => (
            "CLAWBACK".into(),
            json!({
                "asset": format_asset(&op.asset),
                "from": op.from.to_string(),
                "amount": op.amount,
            }),
        ),
        OperationBody::ClawbackClaimableBalance(op) => (
            "CLAWBACK_CLAIMABLE_BALANCE".into(),
            json!({
                "balanceId": format_claimable_balance_id(&op.balance_id),
            }),
        ),
        OperationBody::SetTrustLineFlags(op) => (
            "SET_TRUST_LINE_FLAGS".into(),
            json!({
                "trustor": op.trustor.0.to_string(),
                "asset": format_asset(&op.asset),
                "clearFlags": op.clear_flags,
                "setFlags": op.set_flags,
            }),
        ),
        OperationBody::LiquidityPoolDeposit(op) => (
            "LIQUIDITY_POOL_DEPOSIT".into(),
            json!({
                "liquidityPoolId": hex::encode(op.liquidity_pool_id.0.as_slice()),
                "maxAmountA": op.max_amount_a,
                "maxAmountB": op.max_amount_b,
                "minPrice": format_price(&op.min_price),
                "maxPrice": format_price(&op.max_price),
            }),
        ),
        OperationBody::LiquidityPoolWithdraw(op) => (
            "LIQUIDITY_POOL_WITHDRAW".into(),
            json!({
                "liquidityPoolId": hex::encode(op.liquidity_pool_id.0.as_slice()),
                "amount": op.amount,
                "minAmountA": op.min_amount_a,
                "minAmountB": op.min_amount_b,
            }),
        ),
        OperationBody::InvokeHostFunction(op) => {
            let details = extract_invoke_host_function(op, return_value);
            ("INVOKE_HOST_FUNCTION".into(), details)
        }
        OperationBody::ExtendFootprintTtl(op) => (
            "EXTEND_FOOTPRINT_TTL".into(),
            json!({
                "extendTo": op.extend_to,
            }),
        ),
        OperationBody::RestoreFootprint(_) => ("RESTORE_FOOTPRINT".into(), json!({})),
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
        assert_eq!(result[0].op_type, "PAYMENT");
        assert_eq!(result[0].transaction_hash, "abcd1234");
        assert_eq!(result[0].operation_index, 0);
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
        assert_eq!(result[0].op_type, "CREATE_ACCOUNT");
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
        assert_eq!(result[0].op_type, "INVOKE_HOST_FUNCTION");
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
        assert_eq!(result[0].operation_index, 0);
        assert_eq!(result[0].op_type, "INFLATION");
        assert_eq!(result[1].operation_index, 1);
        assert_eq!(result[1].op_type, "BUMP_SEQUENCE");
        assert_eq!(result[1].details["bumpTo"], 42);
        assert_eq!(result[2].operation_index, 2);
        assert_eq!(result[2].op_type, "END_SPONSORING_FUTURE_RESERVES");
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

        assert_eq!(result[0].op_type, "MANAGE_DATA");
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

        assert_eq!(result[0].op_type, "MANAGE_SELL_OFFER");
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

        assert_eq!(result[0].op_type, "SET_OPTIONS");
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

        assert_eq!(result[0].op_type, "EXTEND_FOOTPRINT_TTL");
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

        assert_eq!(result[0].op_type, "RESTORE_FOOTPRINT");
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
