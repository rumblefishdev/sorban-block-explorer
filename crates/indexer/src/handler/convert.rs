//! Conversion from xdr-parser `Extracted*` types to domain types.
//!
//! Domain types use PostgreSQL-native types (DateTime<Utc>, i64 for sequences)
//! while extracted types use raw u32/i64 timestamps.

use chrono::{DateTime, TimeZone, Utc};
use domain::{
    account::Account,
    ledger::Ledger,
    nft::Nft,
    operation::Operation,
    pool::{LiquidityPool, LiquidityPoolSnapshot},
    soroban::{SorobanContract, SorobanInvocation},
    token::Token,
    transaction::Transaction,
};
use xdr_parser::types::{
    ExtractedAccountState, ExtractedContractDeployment, ExtractedInvocation, ExtractedLedger,
    ExtractedLiquidityPool, ExtractedLiquidityPoolSnapshot, ExtractedNft, ExtractedOperation,
    ExtractedToken, ExtractedTransaction,
};

fn unix_to_datetime(ts: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(ts, 0).single().unwrap_or_default()
}

pub fn to_ledger(e: &ExtractedLedger) -> Ledger {
    Ledger {
        sequence: e.sequence as i64,
        hash: e.hash.clone(),
        closed_at: unix_to_datetime(e.closed_at),
        protocol_version: e.protocol_version as i32,
        transaction_count: e.transaction_count as i32,
        base_fee: e.base_fee as i64,
    }
}

pub fn to_transaction(e: &ExtractedTransaction) -> Transaction {
    Transaction {
        id: 0, // assigned by DB
        hash: e.hash.clone(),
        ledger_sequence: e.ledger_sequence as i64,
        source_account: e.source_account.clone(),
        fee_charged: e.fee_charged,
        successful: e.successful,
        result_code: Some(e.result_code.clone()),
        memo_type: e.memo_type.clone(),
        memo: e.memo.clone(),
        created_at: unix_to_datetime(e.created_at),
        parse_error: Some(e.parse_error),
    }
}

pub fn to_operation(
    e: &ExtractedOperation,
    transaction_id: i64,
    tx_source_account: &str,
) -> Operation {
    Operation {
        id: 0, // assigned by DB
        transaction_id,
        application_order: e.operation_index as i16,
        source_account: e
            .source_account
            .clone()
            .unwrap_or_else(|| tx_source_account.to_string()),
        op_type: e.op_type.clone(),
    }
}

pub fn to_invocation(e: &ExtractedInvocation, transaction_id: i64) -> SorobanInvocation {
    SorobanInvocation {
        id: 0,
        transaction_id,
        contract_id: e.contract_id.clone(),
        caller_account: e.caller_account.clone(),
        function_name: e.function_name.clone().unwrap_or_default(),
        successful: e.successful,
        invocation_index: e.invocation_index as i16,
        ledger_sequence: e.ledger_sequence as i64,
        created_at: unix_to_datetime(e.created_at),
    }
}

pub fn to_contract(e: &ExtractedContractDeployment) -> SorobanContract {
    SorobanContract {
        contract_id: e.contract_id.clone(),
        wasm_hash: e.wasm_hash.clone(),
        deployer_account: e.deployer_account.clone(),
        deployed_at_ledger: Some(e.deployed_at_ledger as i64),
        contract_type: Some(e.contract_type.clone()),
        is_sac: Some(e.is_sac),
        metadata: Some(e.metadata.clone()),
    }
}

pub fn to_account(e: &ExtractedAccountState) -> Account {
    Account {
        account_id: e.account_id.clone(),
        first_seen_ledger: e.first_seen_ledger.unwrap_or(e.last_seen_ledger) as i64,
        last_seen_ledger: e.last_seen_ledger as i64,
        sequence_number: e.sequence_number,
        balances: e.balances.clone(),
        home_domain: e.home_domain.clone(),
    }
}

pub fn to_liquidity_pool(e: &ExtractedLiquidityPool) -> LiquidityPool {
    LiquidityPool {
        pool_id: e.pool_id.clone(),
        asset_a: e.asset_a.clone(),
        asset_b: e.asset_b.clone(),
        fee_bps: e.fee_bps,
        reserves: e.reserves.clone(),
        total_shares: e.total_shares.clone(),
        tvl: e.tvl.clone(),
        created_at_ledger: e.created_at_ledger.unwrap_or(e.last_updated_ledger) as i64,
        last_updated_ledger: e.last_updated_ledger as i64,
    }
}

pub fn to_pool_snapshot(e: &ExtractedLiquidityPoolSnapshot) -> LiquidityPoolSnapshot {
    LiquidityPoolSnapshot {
        id: 0,
        pool_id: e.pool_id.clone(),
        ledger_sequence: e.ledger_sequence as i64,
        created_at: unix_to_datetime(e.created_at),
        reserves: e.reserves.clone(),
        total_shares: e.total_shares.clone(),
        tvl: e.tvl.clone(),
        volume: e.volume.clone(),
        fee_revenue: e.fee_revenue.clone(),
    }
}

pub fn to_token(e: &ExtractedToken) -> Token {
    Token {
        id: 0,
        asset_type: e.asset_type.clone(),
        asset_code: e.asset_code.clone(),
        issuer_address: e.issuer_address.clone(),
        contract_id: e.contract_id.clone(),
        name: e.name.clone(),
        total_supply: e.total_supply.clone(),
        holder_count: e.holder_count,
        metadata: None,
    }
}

pub fn to_nft(e: &ExtractedNft) -> Nft {
    Nft {
        contract_id: e.contract_id.clone(),
        token_id: e.token_id.clone(),
        collection_name: e.collection_name.clone(),
        owner_account: e.owner_account.clone(),
        name: e.name.clone(),
        media_url: e.media_url.clone(),
        metadata: e.metadata.clone(),
        minted_at_ledger: e.minted_at_ledger.map(|l| l as i64),
        last_seen_ledger: e.last_seen_ledger as i64,
    }
}
