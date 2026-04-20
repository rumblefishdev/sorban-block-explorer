//! Persistence layer — currently a stub.
//!
//! The pre-ADR-0027 write-path was removed in task 0148. The signature of
//! `persist_ledger` is preserved so `process_ledger` and `backfill-bench`
//! keep compiling unchanged, but the body is empty until the ADR 0027
//! write-path is wired in the follow-up task.

use tracing::warn;
use xdr_parser::types::{
    ExtractedAccountState, ExtractedContractDeployment, ExtractedContractInterface, ExtractedEvent,
    ExtractedInvocation, ExtractedLedger, ExtractedLiquidityPool, ExtractedLiquidityPoolSnapshot,
    ExtractedNft, ExtractedOperation, ExtractedToken, ExtractedTransaction,
};

use super::HandlerError;

/// Persist all parsed data for a single ledger within `db_tx`.
///
/// The caller is responsible for calling `db_tx.commit()` on success.
#[allow(clippy::too_many_arguments)]
pub async fn persist_ledger(
    db_tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    ledger: &ExtractedLedger,
    transactions: &[ExtractedTransaction],
    operations: &[(String, Vec<ExtractedOperation>)],
    events: &[(String, Vec<ExtractedEvent>)],
    invocations: &[(String, Vec<ExtractedInvocation>)],
    operation_trees: &[(String, serde_json::Value)],
    contract_interfaces: &[ExtractedContractInterface],
    contract_deployments: &[ExtractedContractDeployment],
    account_states: &[ExtractedAccountState],
    liquidity_pools: &[ExtractedLiquidityPool],
    pool_snapshots: &[ExtractedLiquidityPoolSnapshot],
    tokens: &[ExtractedToken],
    nfts: &[ExtractedNft],
) -> Result<(), HandlerError> {
    // TODO(adr-0027-writes): wire new write-path against the ADR 0027 schema.
    // Body intentionally empty — indexer parses but does not persist until the
    // follow-up task replaces this stub. Warn at WARN level so operators don't
    // mistake the unchanged "ledger saved to database" log downstream (or the
    // LastProcessedLedgerSequence CloudWatch metric) for real persistence.
    warn!(
        ledger_sequence = ledger.sequence,
        "persist_ledger is a no-op — persistence disabled until the ADR-0027 write-path lands"
    );
    let _ = (
        db_tx,
        transactions,
        operations,
        events,
        invocations,
        operation_trees,
        contract_interfaces,
        contract_deployments,
        account_states,
        liquidity_pools,
        pool_snapshots,
        tokens,
        nfts,
    );
    Ok(())
}
