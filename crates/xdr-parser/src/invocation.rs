//! Invocation tree extraction from Soroban transaction auth entries.
//!
//! Decodes `SorobanAuthorizedInvocation` trees from `InvokeHostFunctionOp.auth`
//! into both flat `ExtractedInvocation` rows and a nested JSON hierarchy
//! (`operation_tree`) for the transaction detail page.
//!
//! ## Design note: auth entries as invocation source
//!
//! The invocation tree is extracted from **auth entries** (`SorobanAuthorizationEntry.
//! root_invocation` in the transaction envelope), not from diagnostic events in
//! `result_meta_xdr`. Auth entries represent the authorization call graph and are the
//! only reliably available **structured** tree in Soroban transactions.
//!
//! **Limitation:** Invocations that do not require caller authorization (e.g. read-only
//! sub-calls, internal helper contracts) will not appear in the auth tree. For complex
//! DeFi transactions with internal-only sub-calls, the tree may be incomplete. A future
//! enhancement could supplement the auth tree with diagnostic events (`fn_call` /
//! `fn_return`) when available, but diagnostic events depend on protocol configuration
//! and are not guaranteed in production.

use serde_json::{Value, json};
use stellar_xdr::curr::*;

use crate::envelope::InnerTxRef;
use crate::scval::scval_to_typed_json;
use crate::types::ExtractedInvocation;

/// Result of invocation tree extraction.
pub struct InvocationResult {
    /// Flat invocation rows for `soroban_invocations` table.
    pub invocations: Vec<ExtractedInvocation>,
    /// Nested JSON hierarchy for `transactions.operation_tree`.
    /// `None` if the transaction has no Soroban auth entries.
    pub operation_tree: Option<Value>,
}

/// Extract the invocation tree from a transaction envelope's auth entries.
///
/// Scans operations for `InvokeHostFunction` and extracts invocation trees
/// from each auth entry's `root_invocation`. Produces flat rows (depth-first)
/// and a nested JSON tree.
///
/// `successful` is derived from the parent transaction's success status.
/// `tx_meta` is used to populate the root invocation's `return_value` from
/// `SorobanTransactionMeta`; pass `None` if not available.
pub fn extract_invocations(
    envelope: &InnerTxRef<'_>,
    tx_meta: Option<&TransactionMeta>,
    transaction_hash: &str,
    ledger_sequence: u32,
    created_at: i64,
    tx_source_account: &str,
    successful: bool,
) -> InvocationResult {
    let ops = match envelope {
        InnerTxRef::V0(tx) => tx.operations.as_slice(),
        InnerTxRef::V1(tx) => tx.operations.as_slice(),
    };

    let root_return_value = tx_meta
        .and_then(soroban_return_value)
        .map(|v| scval_to_typed_json(&v))
        .unwrap_or(Value::Null);

    let mut ctx = FlattenCtx {
        transaction_hash,
        ledger_sequence,
        created_at,
        successful,
        index: 0,
    };
    let mut all_invocations = Vec::new();
    let mut trees = Vec::new();

    for op in ops {
        if let OperationBody::InvokeHostFunction(ref invoke_op) = op.body {
            // Per-op source_account overrides the tx source (same as extract_operations)
            let caller = op
                .source_account
                .as_ref()
                .map(|a| a.to_string())
                .unwrap_or_else(|| tx_source_account.to_string());

            for auth_entry in invoke_op.auth.iter() {
                let root = &auth_entry.root_invocation;
                let tree_json = invocation_to_json(root, root_return_value.clone(), successful);
                trees.push(tree_json);
                flatten_invocation(
                    &mut ctx,
                    root,
                    Some(caller.clone()),
                    root_return_value.clone(),
                    &mut all_invocations,
                );
            }
        }
    }

    let operation_tree = if trees.is_empty() {
        None
    } else {
        Some(json!(trees))
    };

    InvocationResult {
        invocations: all_invocations,
        operation_tree,
    }
}

/// Shared context for invocation flattening.
struct FlattenCtx<'a> {
    transaction_hash: &'a str,
    ledger_sequence: u32,
    created_at: i64,
    successful: bool,
    index: u32,
}

/// Flatten an invocation tree into `ExtractedInvocation` rows using iterative DFS.
///
/// Uses an explicit stack to avoid stack overflow on deep auth trees
/// (XDR depth limit allows up to ~1000 levels).
fn flatten_invocation(
    ctx: &mut FlattenCtx<'_>,
    root: &SorobanAuthorizedInvocation,
    root_caller: Option<String>,
    root_return_value: Value,
    out: &mut Vec<ExtractedInvocation>,
) {
    struct Frame<'a> {
        node: &'a SorobanAuthorizedInvocation,
        depth: u32,
        caller_account: Option<String>,
        return_value: Value,
    }

    let mut stack = vec![Frame {
        node: root,
        depth: 0,
        caller_account: root_caller,
        return_value: root_return_value,
    }];

    while let Some(frame) = stack.pop() {
        let (contract_id, function_name, function_args) =
            decode_authorized_function(&frame.node.function);

        out.push(ExtractedInvocation {
            transaction_hash: ctx.transaction_hash.to_string(),
            contract_id: contract_id.clone(),
            caller_account: frame.caller_account,
            function_name,
            function_args,
            return_value: frame.return_value,
            successful: ctx.successful,
            invocation_index: ctx.index,
            depth: frame.depth,
            ledger_sequence: ctx.ledger_sequence,
            created_at: ctx.created_at,
        });

        ctx.index += 1;

        // Push children in reverse so left-to-right DFS order is preserved on pop.
        for child in frame.node.sub_invocations.iter().rev() {
            stack.push(Frame {
                node: child,
                depth: frame.depth + 1,
                caller_account: contract_id.clone(),
                return_value: Value::Null,
            });
        }
    }
}

/// Build a nested JSON tree from an invocation node using iterative post-order traversal.
///
/// Uses an explicit stack to avoid stack overflow on deep auth trees.
fn invocation_to_json(
    root: &SorobanAuthorizedInvocation,
    root_return_value: Value,
    successful: bool,
) -> Value {
    // Post-order: process children before parents. Use two passes:
    // 1. DFS to collect nodes in visit order
    // 2. Process in reverse, building children arrays bottom-up

    struct Visit<'a> {
        node: &'a SorobanAuthorizedInvocation,
        return_value: Value,
        child_count: usize,
    }

    let mut visits = Vec::new();
    let mut dfs_stack: Vec<(&SorobanAuthorizedInvocation, Value)> =
        vec![(root, root_return_value)];

    while let Some((node, ret_val)) = dfs_stack.pop() {
        let child_count = node.sub_invocations.len();
        visits.push(Visit {
            node,
            return_value: ret_val,
            child_count,
        });
        // Push children in reverse for left-to-right order
        for child in node.sub_invocations.iter().rev() {
            dfs_stack.push((child, Value::Null));
        }
    }

    // Build JSON bottom-up: process visits in reverse
    let mut result_stack: Vec<Value> = Vec::new();
    for visit in visits.into_iter().rev() {
        let (contract_id, function_name, function_args) =
            decode_authorized_function(&visit.node.function);

        // Pop this node's children from the result stack
        let children: Vec<Value> = result_stack
            .split_off(result_stack.len() - visit.child_count);

        let node_json = json!({
            "contractId": contract_id,
            "functionName": function_name,
            "args": function_args,
            "returnValue": visit.return_value,
            "successful": successful,
            "children": children,
        });
        result_stack.push(node_json);
    }

    result_stack.pop().unwrap_or(Value::Null)
}

/// Decode a `SorobanAuthorizedFunction` into (contract_id, function_name, args_json).
fn decode_authorized_function(
    func: &SorobanAuthorizedFunction,
) -> (Option<String>, Option<String>, Value) {
    match func {
        SorobanAuthorizedFunction::ContractFn(args) => {
            let contract_id = args.contract_address.to_string();
            let function_name = std::str::from_utf8(args.function_name.as_vec())
                .unwrap_or("<invalid-utf8>")
                .to_string();
            let function_args: Vec<Value> = args.args.iter().map(scval_to_typed_json).collect();
            (Some(contract_id), Some(function_name), json!(function_args))
        }
        SorobanAuthorizedFunction::CreateContractHostFn(args) => {
            let executable = format_contract_executable(&args.executable);
            (
                None,
                Some("createContract".to_string()),
                json!({
                    "type": "createContract",
                    "executable": executable,
                }),
            )
        }
        SorobanAuthorizedFunction::CreateContractV2HostFn(args) => {
            let executable = format_contract_executable(&args.executable);
            let constructor_args: Vec<Value> = args
                .constructor_args
                .iter()
                .map(scval_to_typed_json)
                .collect();
            (
                None,
                Some("createContractV2".to_string()),
                json!({
                    "type": "createContractV2",
                    "executable": executable,
                    "constructorArgs": constructor_args,
                }),
            )
        }
    }
}

/// Extract the Soroban return value from transaction metadata, if present.
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

fn format_contract_executable(exec: &ContractExecutable) -> Value {
    match exec {
        ContractExecutable::Wasm(hash) => json!({ "type": "wasm", "hash": hex::encode(hash.0) }),
        ContractExecutable::StellarAsset => json!({ "type": "stellar_asset" }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source_account_str() -> &'static str {
        "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF"
    }

    #[test]
    fn extract_single_invocation() {
        let contract_addr = ScAddress::Contract(ContractId(Hash([0xDD; 32])));
        let func_name = ScSymbol::try_from("transfer".as_bytes().to_vec()).unwrap();
        let args: VecM<ScVal> = vec![ScVal::U64(42)].try_into().unwrap();

        let root = SorobanAuthorizedInvocation {
            function: SorobanAuthorizedFunction::ContractFn(InvokeContractArgs {
                contract_address: contract_addr,
                function_name: func_name,
                args,
            }),
            sub_invocations: VecM::default(),
        };

        let auth_entry = SorobanAuthorizationEntry {
            credentials: SorobanCredentials::SourceAccount,
            root_invocation: root,
        };

        let op = Operation {
            source_account: None,
            body: OperationBody::InvokeHostFunction(InvokeHostFunctionOp {
                host_function: HostFunction::InvokeContract(InvokeContractArgs {
                    contract_address: ScAddress::Contract(ContractId(Hash([0xDD; 32]))),
                    function_name: ScSymbol::try_from("transfer".as_bytes().to_vec()).unwrap(),
                    args: VecM::default(),
                }),
                auth: vec![auth_entry].try_into().unwrap(),
            }),
        };

        let tx = build_v1_tx(vec![op]);
        let inner = InnerTxRef::V1(&tx);
        let result = extract_invocations(
            &inner,
            None,
            "abcd1234",
            100,
            1700000000,
            source_account_str(),
            true,
        );

        assert_eq!(result.invocations.len(), 1);
        let inv = &result.invocations[0];
        assert_eq!(inv.transaction_hash, "abcd1234");
        assert!(inv.contract_id.is_some());
        assert_eq!(inv.function_name.as_deref(), Some("transfer"));
        assert_eq!(inv.caller_account.as_deref(), Some(source_account_str()));
        assert!(inv.return_value.is_null());
        assert_eq!(inv.depth, 0);
        assert_eq!(inv.invocation_index, 0);
        assert!(inv.successful);
        assert_eq!(inv.ledger_sequence, 100);
        assert_eq!(inv.created_at, 1700000000);

        // Check args
        let args = inv.function_args.as_array().unwrap();
        assert_eq!(args.len(), 1);
        assert_eq!(args[0]["type"], "u64");
        assert_eq!(args[0]["value"], 42);

        // Check operation_tree
        let tree = result.operation_tree.unwrap();
        let roots = tree.as_array().unwrap();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0]["functionName"], "transfer");
        assert_eq!(roots[0]["children"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn extract_nested_invocations_with_caller_chain() {
        let child_addr = ScAddress::Contract(ContractId(Hash([0xBB; 32])));
        let root_addr = ScAddress::Contract(ContractId(Hash([0xAA; 32])));

        let child = SorobanAuthorizedInvocation {
            function: SorobanAuthorizedFunction::ContractFn(InvokeContractArgs {
                contract_address: child_addr,
                function_name: ScSymbol::try_from("approve".as_bytes().to_vec()).unwrap(),
                args: VecM::default(),
            }),
            sub_invocations: VecM::default(),
        };

        let root = SorobanAuthorizedInvocation {
            function: SorobanAuthorizedFunction::ContractFn(InvokeContractArgs {
                contract_address: root_addr.clone(),
                function_name: ScSymbol::try_from("swap".as_bytes().to_vec()).unwrap(),
                args: VecM::default(),
            }),
            sub_invocations: vec![child].try_into().unwrap(),
        };

        let auth_entry = SorobanAuthorizationEntry {
            credentials: SorobanCredentials::SourceAccount,
            root_invocation: root,
        };

        let op = Operation {
            source_account: None,
            body: OperationBody::InvokeHostFunction(InvokeHostFunctionOp {
                host_function: HostFunction::InvokeContract(InvokeContractArgs {
                    contract_address: root_addr,
                    function_name: ScSymbol::try_from("swap".as_bytes().to_vec()).unwrap(),
                    args: VecM::default(),
                }),
                auth: vec![auth_entry].try_into().unwrap(),
            }),
        };

        let tx = build_v1_tx(vec![op]);
        let inner = InnerTxRef::V1(&tx);
        let result = extract_invocations(
            &inner,
            None,
            "abcd1234",
            100,
            1700000000,
            source_account_str(),
            true,
        );

        // Flat: 2 rows (root + child), depth-first order
        assert_eq!(result.invocations.len(), 2);

        // Root: caller is the tx source account
        assert_eq!(result.invocations[0].function_name.as_deref(), Some("swap"));
        assert_eq!(result.invocations[0].depth, 0);
        assert_eq!(
            result.invocations[0].caller_account.as_deref(),
            Some(source_account_str())
        );

        // Child: caller is the root's contract_id
        assert_eq!(
            result.invocations[1].function_name.as_deref(),
            Some("approve")
        );
        assert_eq!(result.invocations[1].depth, 1);
        assert_eq!(
            result.invocations[1].caller_account.as_deref(),
            result.invocations[0].contract_id.as_deref()
        );

        // Tree: nested JSON
        let tree = result.operation_tree.unwrap();
        let roots = tree.as_array().unwrap();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0]["functionName"], "swap");
        let children = roots[0]["children"].as_array().unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0]["functionName"], "approve");
    }

    #[test]
    fn root_invocation_gets_return_value_from_meta() {
        let contract_addr = ScAddress::Contract(ContractId(Hash([0xDD; 32])));

        let root = SorobanAuthorizedInvocation {
            function: SorobanAuthorizedFunction::ContractFn(InvokeContractArgs {
                contract_address: contract_addr.clone(),
                function_name: ScSymbol::try_from("get_balance".as_bytes().to_vec()).unwrap(),
                args: VecM::default(),
            }),
            sub_invocations: VecM::default(),
        };

        let auth_entry = SorobanAuthorizationEntry {
            credentials: SorobanCredentials::SourceAccount,
            root_invocation: root,
        };

        let op = Operation {
            source_account: None,
            body: OperationBody::InvokeHostFunction(InvokeHostFunctionOp {
                host_function: HostFunction::InvokeContract(InvokeContractArgs {
                    contract_address: contract_addr,
                    function_name: ScSymbol::try_from("get_balance".as_bytes().to_vec()).unwrap(),
                    args: VecM::default(),
                }),
                auth: vec![auth_entry].try_into().unwrap(),
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
        let result = extract_invocations(
            &inner,
            Some(&tx_meta),
            "abcd1234",
            100,
            1700000000,
            source_account_str(),
            true,
        );

        assert_eq!(result.invocations.len(), 1);
        let inv = &result.invocations[0];
        assert_eq!(inv.return_value["type"], "i128");
        assert_eq!(inv.return_value["value"], "999");

        // Also check the JSON tree has returnValue
        let tree = result.operation_tree.unwrap();
        let root_node = &tree.as_array().unwrap()[0];
        assert_eq!(root_node["returnValue"]["type"], "i128");
    }

    #[test]
    fn no_invocations_for_non_invoke_ops() {
        let op = Operation {
            source_account: None,
            body: OperationBody::Inflation,
        };
        let tx = build_v1_tx(vec![op]);
        let inner = InnerTxRef::V1(&tx);
        let result = extract_invocations(
            &inner,
            None,
            "abcd1234",
            100,
            1700000000,
            source_account_str(),
            true,
        );

        assert!(result.invocations.is_empty());
        assert!(result.operation_tree.is_none());
    }

    #[test]
    fn create_contract_invocation() {
        let root = SorobanAuthorizedInvocation {
            function: SorobanAuthorizedFunction::CreateContractHostFn(CreateContractArgs {
                contract_id_preimage: ContractIdPreimage::Address(ContractIdPreimageFromAddress {
                    address: ScAddress::Contract(ContractId(Hash([0xCC; 32]))),
                    salt: Uint256([0; 32]),
                }),
                executable: ContractExecutable::Wasm(Hash([0xFF; 32])),
            }),
            sub_invocations: VecM::default(),
        };

        let auth_entry = SorobanAuthorizationEntry {
            credentials: SorobanCredentials::SourceAccount,
            root_invocation: root,
        };

        let op = Operation {
            source_account: None,
            body: OperationBody::InvokeHostFunction(InvokeHostFunctionOp {
                host_function: HostFunction::CreateContract(CreateContractArgs {
                    contract_id_preimage: ContractIdPreimage::Address(
                        ContractIdPreimageFromAddress {
                            address: ScAddress::Contract(ContractId(Hash([0xCC; 32]))),
                            salt: Uint256([0; 32]),
                        },
                    ),
                    executable: ContractExecutable::Wasm(Hash([0xFF; 32])),
                }),
                auth: vec![auth_entry].try_into().unwrap(),
            }),
        };

        let tx = build_v1_tx(vec![op]);
        let inner = InnerTxRef::V1(&tx);
        let result = extract_invocations(
            &inner,
            None,
            "abcd1234",
            100,
            1700000000,
            source_account_str(),
            true,
        );

        assert_eq!(result.invocations.len(), 1);
        let inv = &result.invocations[0];
        assert!(inv.contract_id.is_none());
        assert_eq!(inv.function_name.as_deref(), Some("createContract"));
        // caller is still the tx source for root
        assert_eq!(inv.caller_account.as_deref(), Some(source_account_str()));
        assert_eq!(inv.function_args["type"], "createContract");
        assert_eq!(inv.function_args["executable"]["type"], "wasm");
    }

    #[test]
    fn deeply_nested_invocations() {
        // Build a 3-level deep tree: root -> mid -> leaf
        let leaf = SorobanAuthorizedInvocation {
            function: SorobanAuthorizedFunction::ContractFn(InvokeContractArgs {
                contract_address: ScAddress::Contract(ContractId(Hash([0x03; 32]))),
                function_name: ScSymbol::try_from("leaf_fn".as_bytes().to_vec()).unwrap(),
                args: VecM::default(),
            }),
            sub_invocations: VecM::default(),
        };

        let mid = SorobanAuthorizedInvocation {
            function: SorobanAuthorizedFunction::ContractFn(InvokeContractArgs {
                contract_address: ScAddress::Contract(ContractId(Hash([0x02; 32]))),
                function_name: ScSymbol::try_from("mid_fn".as_bytes().to_vec()).unwrap(),
                args: VecM::default(),
            }),
            sub_invocations: vec![leaf].try_into().unwrap(),
        };

        let root = SorobanAuthorizedInvocation {
            function: SorobanAuthorizedFunction::ContractFn(InvokeContractArgs {
                contract_address: ScAddress::Contract(ContractId(Hash([0x01; 32]))),
                function_name: ScSymbol::try_from("root_fn".as_bytes().to_vec()).unwrap(),
                args: VecM::default(),
            }),
            sub_invocations: vec![mid].try_into().unwrap(),
        };

        let auth_entry = SorobanAuthorizationEntry {
            credentials: SorobanCredentials::SourceAccount,
            root_invocation: root,
        };

        let op = Operation {
            source_account: None,
            body: OperationBody::InvokeHostFunction(InvokeHostFunctionOp {
                host_function: HostFunction::InvokeContract(InvokeContractArgs {
                    contract_address: ScAddress::Contract(ContractId(Hash([0x01; 32]))),
                    function_name: ScSymbol::try_from("root_fn".as_bytes().to_vec()).unwrap(),
                    args: VecM::default(),
                }),
                auth: vec![auth_entry].try_into().unwrap(),
            }),
        };

        let tx = build_v1_tx(vec![op]);
        let inner = InnerTxRef::V1(&tx);
        let result = extract_invocations(
            &inner,
            None,
            "abcd1234",
            100,
            1700000000,
            source_account_str(),
            false,
        );

        // Flat: 3 rows with correct caller chain
        assert_eq!(result.invocations.len(), 3);

        // root: caller = tx source
        assert_eq!(result.invocations[0].depth, 0);
        assert_eq!(
            result.invocations[0].function_name.as_deref(),
            Some("root_fn")
        );
        assert!(!result.invocations[0].successful);
        assert_eq!(
            result.invocations[0].caller_account.as_deref(),
            Some(source_account_str())
        );

        // mid: caller = root's contract
        assert_eq!(result.invocations[1].depth, 1);
        assert_eq!(
            result.invocations[1].function_name.as_deref(),
            Some("mid_fn")
        );
        assert_eq!(
            result.invocations[1].caller_account.as_deref(),
            result.invocations[0].contract_id.as_deref()
        );

        // leaf: caller = mid's contract
        assert_eq!(result.invocations[2].depth, 2);
        assert_eq!(
            result.invocations[2].function_name.as_deref(),
            Some("leaf_fn")
        );
        assert_eq!(
            result.invocations[2].caller_account.as_deref(),
            result.invocations[1].contract_id.as_deref()
        );

        // Sub-invocations have null return_value
        assert!(result.invocations[1].return_value.is_null());
        assert!(result.invocations[2].return_value.is_null());

        // Tree: nested 3 levels
        let tree = result.operation_tree.unwrap();
        let root_node = &tree.as_array().unwrap()[0];
        assert_eq!(root_node["functionName"], "root_fn");
        let mid_node = &root_node["children"][0];
        assert_eq!(mid_node["functionName"], "mid_fn");
        let leaf_node = &mid_node["children"][0];
        assert_eq!(leaf_node["functionName"], "leaf_fn");
        assert_eq!(leaf_node["children"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn multiple_auth_entries_produce_multiple_roots() {
        let make_auth = |name: &str| SorobanAuthorizationEntry {
            credentials: SorobanCredentials::SourceAccount,
            root_invocation: SorobanAuthorizedInvocation {
                function: SorobanAuthorizedFunction::ContractFn(InvokeContractArgs {
                    contract_address: ScAddress::Contract(ContractId(Hash([0xAA; 32]))),
                    function_name: ScSymbol::try_from(name.as_bytes().to_vec()).unwrap(),
                    args: VecM::default(),
                }),
                sub_invocations: VecM::default(),
            },
        };

        let op = Operation {
            source_account: None,
            body: OperationBody::InvokeHostFunction(InvokeHostFunctionOp {
                host_function: HostFunction::InvokeContract(InvokeContractArgs {
                    contract_address: ScAddress::Contract(ContractId(Hash([0xAA; 32]))),
                    function_name: ScSymbol::try_from("fn1".as_bytes().to_vec()).unwrap(),
                    args: VecM::default(),
                }),
                auth: vec![make_auth("fn1"), make_auth("fn2")].try_into().unwrap(),
            }),
        };

        let tx = build_v1_tx(vec![op]);
        let inner = InnerTxRef::V1(&tx);
        let result = extract_invocations(
            &inner,
            None,
            "abcd1234",
            100,
            1700000000,
            source_account_str(),
            true,
        );

        assert_eq!(result.invocations.len(), 2);
        assert_eq!(result.invocations[0].invocation_index, 0);
        assert_eq!(result.invocations[1].invocation_index, 1);

        let tree = result.operation_tree.unwrap();
        assert_eq!(tree.as_array().unwrap().len(), 2);
    }

    #[test]
    fn v0_envelope_extracts_invocations() {
        let contract_addr = ScAddress::Contract(ContractId(Hash([0xDD; 32])));

        let auth_entry = SorobanAuthorizationEntry {
            credentials: SorobanCredentials::SourceAccount,
            root_invocation: SorobanAuthorizedInvocation {
                function: SorobanAuthorizedFunction::ContractFn(InvokeContractArgs {
                    contract_address: contract_addr.clone(),
                    function_name: ScSymbol::try_from("hello".as_bytes().to_vec()).unwrap(),
                    args: VecM::default(),
                }),
                sub_invocations: VecM::default(),
            },
        };

        let op = Operation {
            source_account: None,
            body: OperationBody::InvokeHostFunction(InvokeHostFunctionOp {
                host_function: HostFunction::InvokeContract(InvokeContractArgs {
                    contract_address: contract_addr,
                    function_name: ScSymbol::try_from("hello".as_bytes().to_vec()).unwrap(),
                    args: VecM::default(),
                }),
                auth: vec![auth_entry].try_into().unwrap(),
            }),
        };

        let tx = build_v0_tx(vec![op]);
        let inner = InnerTxRef::V0(&tx);
        let result = extract_invocations(
            &inner,
            None,
            "abcd1234",
            100,
            1700000000,
            source_account_str(),
            true,
        );

        assert_eq!(result.invocations.len(), 1);
        assert_eq!(
            result.invocations[0].function_name.as_deref(),
            Some("hello")
        );
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

    fn build_v0_tx(operations: Vec<Operation>) -> TransactionV0 {
        TransactionV0 {
            source_account_ed25519: Uint256([0xAA; 32]),
            fee: 100,
            seq_num: SequenceNumber(1),
            time_bounds: None,
            memo: Memo::None,
            operations: operations.try_into().unwrap(),
            ext: TransactionV0Ext::V0,
        }
    }
}
