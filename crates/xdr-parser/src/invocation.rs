//! Invocation tree extraction from Soroban transaction metadata.
//!
//! Two complementary sources feed `ExtractedInvocation`:
//!
//! 1. **Auth entries** (`SorobanAuthorizationEntry.root_invocation` in the
//!    transaction envelope) — the authorization call graph. Always available
//!    on Soroban transactions, structured (depth + nested args + per-node
//!    function name), but a *subset* of execution: invocations that do not
//!    require caller authorization (read-only sub-calls, contract-authority
//!    sub-calls in DeFi routers) are missing. Used to build the JSON
//!    `operation_tree` returned to the API for the transaction detail page,
//!    where rich per-node detail matters.
//!
//! 2. **Diagnostic events** (`fn_call` / `fn_return` host-VM trace entries
//!    in `*.diagnostic_events`) — the *execution* call graph. Galexie
//!    captive-core emits diagnostic mode by default, so this stream is
//!    reliably present in our ingest. Walked by
//!    [`extract_invocations_from_diagnostics`] and merged into the flat
//!    `ExtractedInvocation` rows that feed the
//!    `soroban_invocations_appearances` appearance index. Closes the
//!    auth-tree coverage gap (~53 % of Soroban tx had zero rows on a local
//!    100-ledger sample — task 0183).
//!
//! Per-row detail (function name, args, return value) for the diagnostic
//! tree is intentionally not persisted — ADR 0029 routes detail to the
//! public archive; the appearance index is goal-of-coverage only.

use serde_json::{Value, json};
use stellar_xdr::curr::*;

use crate::envelope::{InnerTxRef, muxed_to_g_strkey};
use crate::scval::scval_to_typed_json;
use crate::types::ExtractedInvocation;

/// Result of invocation tree extraction.
pub struct InvocationResult {
    /// Flat invocation rows aggregated at indexer staging into
    /// `soroban_invocations_appearances` (ADR 0034).
    pub invocations: Vec<ExtractedInvocation>,
    /// Nested JSON hierarchy for `transactions.operation_tree`.
    /// `None` if the transaction has no Soroban auth entries.
    pub operation_tree: Option<Value>,
}

/// Extract the invocation tree from a transaction envelope's auth entries
/// and execution diagnostics.
///
/// Produces:
/// * `invocations` — flat depth-first rows. When the meta carries diagnostic
///   `fn_call` / `fn_return` events, the **execution** tree (full coverage,
///   contract-to-contract callers included) is the source. Otherwise falls
///   back to the auth-entry tree (subset of execution, but always
///   available). This is what feeds the appearance index.
/// * `operation_tree` — nested JSON hierarchy for `transactions.operation_tree`,
///   built from auth entries only. Per-node function args and return values
///   come straight from XDR, which the host-VM trace does not preserve in a
///   recoverable form for the API consumer. Auth-tree shape is the
///   established read-time contract for the transaction detail page; coverage
///   gaps for that surface are out of scope (ADR 0029 keeps detail in the
///   public archive).
///
/// `successful` is derived from the parent transaction's success status.
/// `tx_meta` is used both to populate the root invocation's `return_value`
/// from `SorobanTransactionMeta` and to surface diagnostic events; pass
/// `None` if not available (auth-tree-only path).
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

    // Always build the JSON `operation_tree` from auth entries. The diagnostic
    // execution tree carries fewer per-node fields (no nested args object the
    // way auth entries do), and the operation_tree shape is already the
    // contract for `transactions.operation_tree`.
    let mut trees = Vec::new();
    for op in ops {
        if let OperationBody::InvokeHostFunction(ref invoke_op) = op.body {
            for auth_entry in invoke_op.auth.iter() {
                let tree_json = invocation_to_json(
                    &auth_entry.root_invocation,
                    root_return_value.clone(),
                    successful,
                );
                trees.push(tree_json);
            }
        }
    }
    let operation_tree = if trees.is_empty() {
        None
    } else {
        Some(json!(trees))
    };

    // Diagnostic-event execution tree (preferred — superset of auth tree).
    // When the meta carries no diagnostic events, fall back to the auth tree.
    //
    // Effective root caller honours per-op `source_account` overrides (matching
    // `flatten_auth_tree`'s task-0177 canonicalisation): muxed M-strkey →
    // underlying ed25519 G-strkey, op override beats tx source. Protocol 21+
    // allows at most one InvokeHostFunction op per tx, so the first match
    // covers every real-world case; if none exists, the tx source is the
    // legitimate fallback.
    let root_caller = ops
        .iter()
        .find_map(|op| match op.body {
            OperationBody::InvokeHostFunction(_) => Some(
                op.source_account
                    .as_ref()
                    .map(muxed_to_g_strkey)
                    .unwrap_or_else(|| tx_source_account.to_string()),
            ),
            _ => None,
        })
        .unwrap_or_else(|| tx_source_account.to_string());

    let diag_invocations = tx_meta
        .map(|tm| {
            extract_invocations_from_diagnostics(
                tm,
                transaction_hash,
                ledger_sequence,
                created_at,
                &root_caller,
                successful,
            )
        })
        .unwrap_or_default();

    let invocations = if diag_invocations.is_empty() {
        flatten_auth_tree(
            ops,
            tx_source_account,
            transaction_hash,
            ledger_sequence,
            created_at,
            successful,
            root_return_value,
        )
    } else {
        diag_invocations
    };

    InvocationResult {
        invocations,
        operation_tree,
    }
}

/// Auth-tree fallback path. Same shape as the original pre-task-0183
/// `extract_invocations` body — preserved as-is for transactions that have
/// no diagnostic events at all (degenerate/Protocol-22 cases). Auth-tree
/// coverage matches its long-standing semantic: subset of execution, root
/// caller is the per-op source account.
fn flatten_auth_tree(
    ops: &[Operation],
    tx_source_account: &str,
    transaction_hash: &str,
    ledger_sequence: u32,
    created_at: i64,
    successful: bool,
    root_return_value: Value,
) -> Vec<ExtractedInvocation> {
    let mut ctx = FlattenCtx {
        transaction_hash,
        ledger_sequence,
        created_at,
        successful,
        index: 0,
    };
    let mut out = Vec::new();
    for op in ops {
        if let OperationBody::InvokeHostFunction(ref invoke_op) = op.body {
            // Per-op source_account overrides the tx source (same as extract_operations).
            // Canonicalize muxed → underlying ed25519 G-strkey so callers see the
            // same 56-char form they'd see for a non-muxed source. See task 0177.
            let caller = op
                .source_account
                .as_ref()
                .map(muxed_to_g_strkey)
                .unwrap_or_else(|| tx_source_account.to_string());

            for auth_entry in invoke_op.auth.iter() {
                flatten_invocation(
                    &mut ctx,
                    &auth_entry.root_invocation,
                    Some(caller.clone()),
                    root_return_value.clone(),
                    &mut out,
                );
            }
        }
    }
    out
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

/// Extract the **execution** invocation tree from `fn_call` / `fn_return`
/// host-VM diagnostic events.
///
/// The host emits a depth-first stream around every contract entry/exit:
///
/// * `fn_call`  — `topics = [Symbol("fn_call"), Address(contract_to_call),
///                Symbol(function_name)]`, `data = Vec(args)`. The event's
///   `contract_id` field is `None` (host event); the called contract lives
///   in `topics[1]`.
/// * `fn_return` — `topics = [Symbol("fn_return"), Symbol(function_name)]`,
///   `data = ScVal(return_value)`. The event's `contract_id` field carries
///   the contract that's returning; we use it to validate the stack pop
///   but never depend on it for correctness.
///
/// Walking the stream:
///
/// 1. On `fn_call`, push a frame and emit an `ExtractedInvocation` whose
///    `caller` is the contract on top of the active stack (the caller is
///    "the frame currently executing"). When the stack is empty — the
///    very first call of the tx, or the tx-source-account-rooted root —
///    the caller is `tx_source_account`.
/// 2. On `fn_return`, pop the topmost frame.
/// 3. On execution traps that leave residual unmatched calls, the
///    remaining frames are silently dropped at end-of-stream — they were
///    already emitted on their `fn_call`.
///
/// Returns an empty Vec when the meta has no diagnostic events
/// (signalling the auth-tree fallback path in the caller).
///
/// The caller chain is the contract `C…` StrKey, not an account `G…` —
/// the indexer staging layer routes contract callers to
/// `caller_contract_id` (task 0183 schema) while keeping account callers
/// on the existing `caller_id` column.
pub fn extract_invocations_from_diagnostics(
    tx_meta: &TransactionMeta,
    transaction_hash: &str,
    ledger_sequence: u32,
    created_at: i64,
    tx_source_account: &str,
    successful: bool,
) -> Vec<ExtractedInvocation> {
    let diags = collect_diagnostic_events(tx_meta);
    if diags.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut stack: Vec<DiagFrame> = Vec::new();
    let mut index: u32 = 0;

    for diag in diags {
        // Diagnostic-typed only — host-VM trace entries. Contract-typed
        // copies of consensus events live alongside in v4.diagnostic_events
        // (Galexie diagnostic mode) and must not be misread as fn_call /
        // fn_return.
        if !matches!(diag.event.type_, ContractEventType::Diagnostic) {
            continue;
        }
        let ContractEventBody::V0(ref v0) = diag.event.body;

        let Some(kind) = classify_diag_topic(&v0.topics) else {
            continue;
        };

        match kind {
            DiagKind::FnCall { contract_id } => {
                let caller = match stack.last() {
                    Some(frame) => frame.contract_id.clone(),
                    None => tx_source_account.to_string(),
                };
                out.push(ExtractedInvocation {
                    transaction_hash: transaction_hash.to_string(),
                    contract_id: Some(contract_id.clone()),
                    caller_account: Some(caller),
                    function_name: None,
                    function_args: Value::Null,
                    return_value: Value::Null,
                    successful,
                    invocation_index: index,
                    depth: stack.len() as u32,
                    ledger_sequence,
                    created_at,
                });
                index += 1;
                stack.push(DiagFrame { contract_id });
            }
            DiagKind::FnReturn => {
                stack.pop();
            }
        }
    }

    out
}

struct DiagFrame {
    contract_id: String,
}

enum DiagKind {
    FnCall { contract_id: String },
    FnReturn,
}

fn classify_diag_topic(topics: &VecM<ScVal>) -> Option<DiagKind> {
    // Topic 0 distinguishes the kind. Anything other than fn_call/fn_return
    // is a different host trace (core_metrics, log, error, host_fn_failed)
    // — not part of the call graph.
    let head_sym = match topics.first()? {
        ScVal::Symbol(sym) => std::str::from_utf8(sym.as_vec()).ok()?,
        _ => return None,
    };

    match head_sym {
        "fn_call" => Some(DiagKind::FnCall {
            contract_id: decode_call_target(topics.get(1)?)?,
        }),
        "fn_return" => Some(DiagKind::FnReturn),
        _ => None,
    }
}

/// Decode the called-contract identity from a `fn_call` event's
/// `topics[1]`. The host historically encodes this two ways:
///
/// * `ScVal::Bytes` carrying the raw 32-byte contract hash — this is
///   what mainnet captive-core actually emits today (verified against
///   ledger 62016086 on 2026-04-30).
/// * `ScVal::Address(ScAddress::Contract(_))` — the structured form some
///   newer host revisions use. Accepted for forward compatibility so
///   the walker keeps working through future upgrades.
///
/// Returns `None` for any other shape (incl. account `Address` variants
/// — fn_call always targets a contract; an account topic indicates
/// either a malformed event or an unrelated diagnostic kind).
fn decode_call_target(topic: &ScVal) -> Option<String> {
    match topic {
        ScVal::Bytes(bytes) => {
            let raw = bytes.as_slice();
            if raw.len() != 32 {
                return None;
            }
            let mut buf = [0u8; 32];
            buf.copy_from_slice(raw);
            Some(ScAddress::Contract(ContractId(Hash(buf))).to_string())
        }
        ScVal::Address(addr @ ScAddress::Contract(_)) => Some(addr.to_string()),
        _ => None,
    }
}

/// Pull `diagnostic_events` from V3 (`soroban_meta.diagnostic_events`) or
/// V4 (`v4.diagnostic_events`) meta. Galexie's captive-core enables
/// diagnostic mode by default, so the V4 stream is reliably populated;
/// the V3 path is kept for parity with `extract_events`.
fn collect_diagnostic_events(meta: &TransactionMeta) -> Vec<&DiagnosticEvent> {
    match meta {
        TransactionMeta::V3(v3) => v3
            .soroban_meta
            .as_ref()
            .map(|m| m.diagnostic_events.iter().collect())
            .unwrap_or_default(),
        TransactionMeta::V4(v4) => v4.diagnostic_events.iter().collect(),
        _ => Vec::new(),
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
    let mut dfs_stack: Vec<(&SorobanAuthorizedInvocation, Value)> = vec![(root, root_return_value)];

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
        let children: Vec<Value> = result_stack.split_off(result_stack.len() - visit.child_count);

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

    /// Bug 0177 regression: a per-op `source_account` override carrying a
    /// `MuxedAccount::MuxedEd25519` variant used to surface a 69-char M-strkey
    /// as `caller_account`, overflowing `accounts.account_id VARCHAR(56)` at
    /// persist time. The override path must canonicalize to the 56-char
    /// underlying ed25519 G-strkey, identical to the bare-ed25519 case.
    #[test]
    fn per_op_muxed_source_collapses_to_underlying_g_strkey() {
        let payload = [0x77; 32];
        let mux_id = 0x0123_4567_89AB_CDEFu64;
        let muxed_source = MuxedAccount::MuxedEd25519(stellar_xdr::curr::MuxedAccountMed25519 {
            id: mux_id,
            ed25519: Uint256(payload),
        });
        let expected_g = MuxedAccount::Ed25519(Uint256(payload)).to_string();
        // Sanity on the test premise: the muxed form alone *would* surface as
        // a 69-char M-strkey if the parser shipped it through unchanged.
        assert_eq!(muxed_source.to_string().len(), 69);
        assert!(muxed_source.to_string().starts_with('M'));
        assert_eq!(expected_g.len(), 56);
        assert!(expected_g.starts_with('G'));

        let auth_entry = SorobanAuthorizationEntry {
            credentials: SorobanCredentials::SourceAccount,
            root_invocation: SorobanAuthorizedInvocation {
                function: SorobanAuthorizedFunction::ContractFn(InvokeContractArgs {
                    contract_address: ScAddress::Contract(ContractId(Hash([0xCC; 32]))),
                    function_name: ScSymbol::try_from("transfer".as_bytes().to_vec()).unwrap(),
                    args: VecM::default(),
                }),
                sub_invocations: VecM::default(),
            },
        };
        let op = Operation {
            // The per-op override is the field under test — distinct from the
            // tx-level source which `tx_source_account` carries below.
            source_account: Some(muxed_source),
            body: OperationBody::InvokeHostFunction(InvokeHostFunctionOp {
                host_function: HostFunction::InvokeContract(InvokeContractArgs {
                    contract_address: ScAddress::Contract(ContractId(Hash([0xCC; 32]))),
                    function_name: ScSymbol::try_from("transfer".as_bytes().to_vec()).unwrap(),
                    args: VecM::default(),
                }),
                auth: vec![auth_entry].try_into().unwrap(),
            }),
        };
        let tx = build_v1_tx(vec![op]);
        let inner = InnerTxRef::V1(&tx);
        // tx_source_account is intentionally distinct — if the per-op override
        // logic regresses to "always fall back to tx source", the assertion
        // below will catch it.
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
        let caller = result.invocations[0]
            .caller_account
            .as_deref()
            .expect("caller_account populated");
        assert_eq!(caller.len(), 56, "expected 56-char G-strkey, got {caller}");
        assert!(caller.starts_with('G'), "expected G prefix, got {caller}");
        assert_eq!(
            caller, expected_g,
            "caller_account must canonicalize to the underlying ed25519 G-strkey"
        );
        assert_ne!(
            caller,
            source_account_str(),
            "caller must come from per-op override (canonicalized), not from tx source fallback"
        );
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

    // -------------------------------------------------------------------
    // Diagnostic-event execution-tree walker (task 0183)
    // -------------------------------------------------------------------

    fn diag_event(topics: Vec<ScVal>, returning_contract: Option<u8>) -> DiagnosticEvent {
        // Host emits the trace as a Diagnostic-typed ContractEvent. fn_call
        // events have contract_id=None (host event); fn_return carries the
        // returning contract on `event.contract_id`. The walker doesn't
        // depend on the latter — kept here just so fixtures match the
        // shape the host produces.
        DiagnosticEvent {
            in_successful_contract_call: true,
            event: ContractEvent {
                ext: ExtensionPoint::V0,
                contract_id: returning_contract.map(|b| ContractId(Hash([b; 32]))),
                type_: ContractEventType::Diagnostic,
                body: ContractEventBody::V0(ContractEventV0 {
                    topics: topics.try_into().unwrap(),
                    data: ScVal::Void,
                }),
            },
        }
    }

    fn fn_call_topics(contract_byte: u8, fn_name: &str) -> Vec<ScVal> {
        vec![
            ScVal::Symbol(ScSymbol::try_from("fn_call".as_bytes().to_vec()).unwrap()),
            ScVal::Address(ScAddress::Contract(ContractId(Hash([contract_byte; 32])))),
            ScVal::Symbol(ScSymbol::try_from(fn_name.as_bytes().to_vec()).unwrap()),
        ]
    }

    /// Mainnet captive-core emits the called contract as raw 32-byte
    /// `ScVal::Bytes`, not as a structured `ScVal::Address`. This shape
    /// must round-trip too.
    fn fn_call_topics_bytes_form(contract_byte: u8, fn_name: &str) -> Vec<ScVal> {
        vec![
            ScVal::Symbol(ScSymbol::try_from("fn_call".as_bytes().to_vec()).unwrap()),
            ScVal::Bytes(ScBytes(vec![contract_byte; 32].try_into().unwrap())),
            ScVal::Symbol(ScSymbol::try_from(fn_name.as_bytes().to_vec()).unwrap()),
        ]
    }

    fn fn_return_topics(fn_name: &str) -> Vec<ScVal> {
        vec![
            ScVal::Symbol(ScSymbol::try_from("fn_return".as_bytes().to_vec()).unwrap()),
            ScVal::Symbol(ScSymbol::try_from(fn_name.as_bytes().to_vec()).unwrap()),
        ]
    }

    fn meta_v4_with_diags(diags: Vec<DiagnosticEvent>) -> TransactionMeta {
        TransactionMeta::V4(TransactionMetaV4 {
            ext: ExtensionPoint::V0,
            tx_changes_before: LedgerEntryChanges::default(),
            operations: VecM::default(),
            tx_changes_after: LedgerEntryChanges::default(),
            soroban_meta: None,
            events: VecM::default(),
            diagnostic_events: diags.try_into().unwrap(),
        })
    }

    /// Empty diagnostic stream → walker returns empty Vec → caller falls
    /// back to the auth tree. Smoke test guarding the fallback contract.
    #[test]
    fn diag_walker_empty_meta_returns_empty() {
        let meta = meta_v4_with_diags(Vec::new());
        let invs = extract_invocations_from_diagnostics(
            &meta,
            "abcd1234",
            100,
            1700000000,
            source_account_str(),
            true,
        );
        assert!(invs.is_empty());
    }

    /// Single fn_call/fn_return pair: one row, caller = tx source, depth 0.
    #[test]
    fn diag_walker_single_call_returns_one_row() {
        let diags = vec![
            diag_event(fn_call_topics(0xCC, "transfer"), None),
            diag_event(fn_return_topics("transfer"), Some(0xCC)),
        ];
        let meta = meta_v4_with_diags(diags);
        let invs = extract_invocations_from_diagnostics(
            &meta,
            "abcd1234",
            100,
            1700000000,
            source_account_str(),
            true,
        );

        assert_eq!(invs.len(), 1);
        let inv = &invs[0];
        assert_eq!(inv.depth, 0);
        assert_eq!(inv.invocation_index, 0);
        assert!(inv.contract_id.as_deref().unwrap().starts_with('C'));
        assert_eq!(inv.caller_account.as_deref(), Some(source_account_str()));
    }

    /// Nested router → pool sub-call: 2 rows, second row's caller is the
    /// router contract (C-prefix), proving the contract-to-contract
    /// caller chain that the auth tree cannot represent.
    #[test]
    fn diag_walker_nested_call_chains_contract_caller() {
        let router = 0xAA;
        let pool = 0xBB;
        let diags = vec![
            diag_event(fn_call_topics(router, "swap"), None),
            diag_event(fn_call_topics(pool, "swap_exact_in"), None),
            diag_event(fn_return_topics("swap_exact_in"), Some(pool)),
            diag_event(fn_return_topics("swap"), Some(router)),
        ];
        let meta = meta_v4_with_diags(diags);
        let invs = extract_invocations_from_diagnostics(
            &meta,
            "abcd1234",
            100,
            1700000000,
            source_account_str(),
            true,
        );

        assert_eq!(invs.len(), 2);

        // Root: router, called by tx source (G-account).
        assert_eq!(invs[0].depth, 0);
        assert_eq!(
            invs[0].caller_account.as_deref(),
            Some(source_account_str())
        );
        let router_id = invs[0].contract_id.clone().expect("router contract id");
        assert!(router_id.starts_with('C'));

        // Child: pool, called by the router (C-prefix). This is the row
        // that the auth tree would never produce for an auth-less router.
        assert_eq!(invs[1].depth, 1);
        assert_eq!(invs[1].caller_account.as_deref(), Some(router_id.as_str()));
        let pool_id = invs[1].contract_id.clone().expect("pool contract id");
        assert!(pool_id.starts_with('C'));
        assert_ne!(pool_id, router_id, "pool and router must be distinct");
    }

    /// Trap mid-call: extra `fn_call` with no matching `fn_return`. Walker
    /// must still emit a row for the trapping frame and not panic on the
    /// residual stack at end-of-stream.
    #[test]
    fn diag_walker_unbalanced_call_does_not_panic() {
        let diags = vec![
            diag_event(fn_call_topics(0xAA, "outer"), None),
            diag_event(fn_call_topics(0xBB, "inner"), None),
            // No fn_return — trap.
        ];
        let meta = meta_v4_with_diags(diags);
        let invs = extract_invocations_from_diagnostics(
            &meta,
            "abcd1234",
            100,
            1700000000,
            source_account_str(),
            true,
        );
        assert_eq!(invs.len(), 2, "every fn_call emits a row, even on trap");
    }

    /// Non-Diagnostic-typed events in `diagnostic_events` (Contract-typed
    /// copies of consensus events under Galexie diagnostic mode — task
    /// 0182) must not be misread as fn_call / fn_return frames.
    #[test]
    fn diag_walker_skips_non_diagnostic_typed_events() {
        let consensus_copy = DiagnosticEvent {
            in_successful_contract_call: true,
            event: ContractEvent {
                ext: ExtensionPoint::V0,
                contract_id: Some(ContractId(Hash([0xCC; 32]))),
                type_: ContractEventType::Contract,
                body: ContractEventBody::V0(ContractEventV0 {
                    topics: vec![ScVal::Symbol(
                        ScSymbol::try_from("transfer".as_bytes().to_vec()).unwrap(),
                    )]
                    .try_into()
                    .unwrap(),
                    data: ScVal::Void,
                }),
            },
        };
        let real_call = diag_event(fn_call_topics(0xCC, "transfer"), None);
        let real_return = diag_event(fn_return_topics("transfer"), Some(0xCC));
        let meta = meta_v4_with_diags(vec![consensus_copy, real_call, real_return]);

        let invs = extract_invocations_from_diagnostics(
            &meta,
            "abcd1234",
            100,
            1700000000,
            source_account_str(),
            true,
        );
        assert_eq!(invs.len(), 1, "only the real fn_call should produce a row");
    }

    /// Mainnet captive-core encodes the called contract in `topics[1]` as
    /// `ScVal::Bytes` (32-byte hash), not as a structured `ScVal::Address`.
    /// Verified against ledger 62016086 (tx b7b510…3235) on 2026-04-30.
    /// A regression here means real production diag streams stop yielding
    /// invocations entirely, while the synthetic Address-form tests above
    /// would keep passing — pin both shapes explicitly.
    #[test]
    fn diag_walker_decodes_bytes_form_call_target() {
        let diags = vec![
            diag_event(fn_call_topics_bytes_form(0xCC, "transfer"), None),
            diag_event(fn_return_topics("transfer"), Some(0xCC)),
        ];
        let meta = meta_v4_with_diags(diags);
        let invs = extract_invocations_from_diagnostics(
            &meta,
            "abcd1234",
            100,
            1700000000,
            source_account_str(),
            true,
        );

        assert_eq!(invs.len(), 1);
        let cid = invs[0]
            .contract_id
            .as_deref()
            .expect("contract id resolved from Bytes form");
        assert!(cid.starts_with('C'));
        assert_eq!(cid.len(), 56);
        // Same bytes via Address form must produce the same StrKey.
        let addr_form_cid = ScAddress::Contract(ContractId(Hash([0xCC; 32]))).to_string();
        assert_eq!(cid, addr_form_cid);
    }

    /// Other Diagnostic-typed traces (`core_metrics`, `log`, `error`,
    /// `host_fn_failed`) sit alongside fn_call / fn_return in the same
    /// container. They have neither shape and must be ignored.
    #[test]
    fn diag_walker_skips_other_diagnostic_topics() {
        let core_metrics = diag_event(
            vec![
                ScVal::Symbol(ScSymbol::try_from("core_metrics".as_bytes().to_vec()).unwrap()),
                ScVal::Symbol(ScSymbol::try_from("cpu_insn".as_bytes().to_vec()).unwrap()),
            ],
            None,
        );
        let real_call = diag_event(fn_call_topics(0xCC, "transfer"), None);
        let real_return = diag_event(fn_return_topics("transfer"), Some(0xCC));
        let meta = meta_v4_with_diags(vec![core_metrics, real_call, real_return]);

        let invs = extract_invocations_from_diagnostics(
            &meta,
            "abcd1234",
            100,
            1700000000,
            source_account_str(),
            true,
        );
        assert_eq!(invs.len(), 1);
    }

    /// Top-level `extract_invocations` must prefer the diag-tree when
    /// present. Without diag, the auth tree is used. With diag, the auth
    /// tree's callers do NOT appear in `invocations` — only the execution
    /// rows. Guards against accidental double-counting from merging both.
    #[test]
    fn extract_invocations_prefers_diag_tree_over_auth() {
        let contract_addr = ScAddress::Contract(ContractId(Hash([0xCC; 32])));

        let auth = SorobanAuthorizationEntry {
            credentials: SorobanCredentials::SourceAccount,
            root_invocation: SorobanAuthorizedInvocation {
                function: SorobanAuthorizedFunction::ContractFn(InvokeContractArgs {
                    contract_address: contract_addr.clone(),
                    function_name: ScSymbol::try_from("transfer".as_bytes().to_vec()).unwrap(),
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
                    function_name: ScSymbol::try_from("transfer".as_bytes().to_vec()).unwrap(),
                    args: VecM::default(),
                }),
                auth: vec![auth].try_into().unwrap(),
            }),
        };
        let tx = build_v1_tx(vec![op]);
        let inner = InnerTxRef::V1(&tx);

        // Diag stream: TWO frames (router + pool) — coverage that auth
        // tree alone would miss. Asserting len == 2 (not 3 = auth + diag)
        // proves the diag path replaces, not augments.
        let diags = vec![
            diag_event(fn_call_topics(0xAA, "swap"), None),
            diag_event(fn_call_topics(0xBB, "pool_swap"), None),
            diag_event(fn_return_topics("pool_swap"), Some(0xBB)),
            diag_event(fn_return_topics("swap"), Some(0xAA)),
        ];
        let meta = meta_v4_with_diags(diags);

        let result = extract_invocations(
            &inner,
            Some(&meta),
            "abcd1234",
            100,
            1700000000,
            source_account_str(),
            true,
        );
        assert_eq!(
            result.invocations.len(),
            2,
            "diag tree replaces auth tree — no double-count"
        );
        // operation_tree still comes from auth entries (auth-tree shape is
        // the established API contract for transactions.operation_tree).
        let tree = result.operation_tree.expect("operation_tree from auth");
        assert_eq!(tree.as_array().unwrap().len(), 1);
    }

    /// Regression for PR #148 review: when an `InvokeHostFunction` op carries
    /// a `source_account` override (and especially a `MuxedAccount::MuxedEd25519`),
    /// the diag-tree root caller must use the canonicalised G-strkey from
    /// THAT op, not the bare tx source. Mirrors the auth-tree counterpart
    /// `per_op_muxed_source_collapses_to_underlying_g_strkey` (task 0177)
    /// for the diagnostic-event path.
    #[test]
    fn diag_root_caller_honours_per_op_source_and_canonicalises_muxed() {
        let payload = [0x77; 32];
        let mux_id = 0x0123_4567_89AB_CDEFu64;
        let muxed_source = MuxedAccount::MuxedEd25519(stellar_xdr::curr::MuxedAccountMed25519 {
            id: mux_id,
            ed25519: Uint256(payload),
        });
        let expected_g = MuxedAccount::Ed25519(Uint256(payload)).to_string();
        // Sanity on the test premise — the muxed form alone would surface as
        // a 69-char M-strkey if anything in the path forgot to canonicalise.
        assert_eq!(muxed_source.to_string().len(), 69);
        assert_eq!(expected_g.len(), 56);

        let op = Operation {
            // Per-op override is the field under test.
            source_account: Some(muxed_source),
            body: OperationBody::InvokeHostFunction(InvokeHostFunctionOp {
                host_function: HostFunction::InvokeContract(InvokeContractArgs {
                    contract_address: ScAddress::Contract(ContractId(Hash([0xCC; 32]))),
                    function_name: ScSymbol::try_from("transfer".as_bytes().to_vec()).unwrap(),
                    args: VecM::default(),
                }),
                auth: VecM::default(),
            }),
        };
        let tx = build_v1_tx(vec![op]);
        let inner = InnerTxRef::V1(&tx);

        let diags = vec![
            diag_event(fn_call_topics(0xCC, "transfer"), None),
            diag_event(fn_return_topics("transfer"), Some(0xCC)),
        ];
        let meta = meta_v4_with_diags(diags);

        // tx_source_account intentionally distinct from the per-op override —
        // if the diag walker regresses to using tx source, the assertion
        // below catches it.
        let result = extract_invocations(
            &inner,
            Some(&meta),
            "abcd1234",
            100,
            1700000000,
            source_account_str(),
            true,
        );

        assert_eq!(result.invocations.len(), 1, "expected single root frame");
        let caller = result.invocations[0]
            .caller_account
            .as_deref()
            .expect("root caller populated");
        assert_eq!(caller.len(), 56, "expected 56-char G-strkey, got {caller}");
        assert!(caller.starts_with('G'));
        assert_eq!(
            caller, expected_g,
            "root caller must canonicalise to underlying ed25519 G-strkey from per-op override"
        );
        assert_ne!(
            caller,
            source_account_str(),
            "root caller must come from per-op override, not tx-source fallback"
        );
    }
}
