//! DB writes for the ADR 0027 write-path.
//!
//! One function per table (or per tightly-coupled table group). Every write
//! uses UNNEST batching — one round trip per table, or one per 5000-row chunk.
//!
//! PG's 65535 bind-parameter limit at ~10 columns caps safe UNNEST at ~6500
//! rows; `CHUNK_SIZE = 5000` keeps headroom.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use domain::{AssetType, ContractType, NftEventType, OperationType, TokenAssetType};
use serde_json::Value;
use sqlx::{Postgres, Transaction};

use super::HandlerError;
use super::classification_cache::ClassificationCache;
use super::staging::{BalanceRow, Staged, TokenRow, TxRow, WasmRow};

const CHUNK_SIZE: usize = 5000;

// ---------------------------------------------------------------------------
// 1. accounts — upsert + RETURNING surrogate id
// ---------------------------------------------------------------------------

/// Upsert every StrKey referenced in this ledger and return the StrKey → id map.
///
/// `last_seen_ledger` is watermark-guarded via `GREATEST`. `sequence_number`
/// and `home_domain` are only overwritten when the incoming ledger is strictly
/// newer than what's already stored — an older replay cannot roll state back.
pub(super) async fn upsert_accounts(
    db_tx: &mut Transaction<'_, Postgres>,
    staged: &Staged,
) -> Result<HashMap<String, i64>, HandlerError> {
    let mut out: HashMap<String, i64> = HashMap::with_capacity(staged.account_keys.len());
    if staged.account_keys.is_empty() {
        return Ok(out);
    }

    for chunk in staged.account_keys.chunks(CHUNK_SIZE) {
        let mut keys: Vec<String> = Vec::with_capacity(chunk.len());
        let mut first_seen: Vec<i64> = Vec::with_capacity(chunk.len());
        let mut last_seen: Vec<i64> = Vec::with_capacity(chunk.len());
        // Sentinel -1 means "no state override for this reference-only account";
        // the SQL coalesces it to 0 for new rows and leaves the existing value
        // untouched on the UPDATE path.
        let mut seq_nums: Vec<i64> = Vec::with_capacity(chunk.len());
        let mut home_domains: Vec<Option<String>> = Vec::with_capacity(chunk.len());

        for k in chunk {
            keys.push(k.clone());
            last_seen.push(staged.ledger_sequence_i64);
            match staged.account_state_overrides.get(k.as_str()) {
                Some(ov) => {
                    first_seen.push(ov.first_seen_ledger.unwrap_or(staged.ledger_sequence_i64));
                    seq_nums.push(ov.sequence_number);
                    home_domains.push(ov.home_domain.clone());
                }
                None => {
                    first_seen.push(staged.ledger_sequence_i64);
                    seq_nums.push(-1);
                    home_domains.push(None);
                }
            }
        }

        let rows: Vec<(i64, String)> = sqlx::query_as(
            r#"
            INSERT INTO accounts (account_id, first_seen_ledger, last_seen_ledger, sequence_number, home_domain)
            SELECT ak, fs, ls, COALESCE(NULLIF(sq, -1), 0), hd
              FROM UNNEST($1::VARCHAR[], $2::BIGINT[], $3::BIGINT[], $4::BIGINT[], $5::VARCHAR[])
                AS t(ak, fs, ls, sq, hd)
            ON CONFLICT (account_id) DO UPDATE
              SET last_seen_ledger = GREATEST(accounts.last_seen_ledger, EXCLUDED.last_seen_ledger),
                  sequence_number  = CASE
                      WHEN EXCLUDED.last_seen_ledger >= accounts.last_seen_ledger
                       AND EXCLUDED.sequence_number <> -1
                      THEN EXCLUDED.sequence_number
                      ELSE accounts.sequence_number
                  END,
                  home_domain = CASE
                      WHEN EXCLUDED.last_seen_ledger >= accounts.last_seen_ledger
                       AND EXCLUDED.home_domain IS NOT NULL
                      THEN EXCLUDED.home_domain
                      ELSE accounts.home_domain
                  END,
                  first_seen_ledger = LEAST(accounts.first_seen_ledger, EXCLUDED.first_seen_ledger)
            RETURNING id, account_id
            "#,
        )
        .bind(&keys)
        .bind(&first_seen)
        .bind(&last_seen)
        .bind(&seq_nums)
        .bind(&home_domains)
        .fetch_all(&mut **db_tx)
        .await?;

        for (id, key) in rows {
            out.insert(key, id);
        }
    }

    Ok(out)
}

// ---------------------------------------------------------------------------
// 2. wasm_interface_metadata — upsert
// ---------------------------------------------------------------------------

pub(super) async fn upsert_wasm_metadata(
    db_tx: &mut Transaction<'_, Postgres>,
    staged: &Staged,
) -> Result<(), HandlerError> {
    if staged.wasm_rows.is_empty() {
        return Ok(());
    }
    for chunk in staged.wasm_rows.chunks(CHUNK_SIZE) {
        let hashes: Vec<Vec<u8>> = chunk.iter().map(|r| r.wasm_hash.to_vec()).collect();
        let metadatas: Vec<Value> = chunk.iter().map(|r: &WasmRow| r.metadata.clone()).collect();
        sqlx::query(
            r#"
            INSERT INTO wasm_interface_metadata (wasm_hash, metadata)
            SELECT wh, md
              FROM UNNEST($1::BYTEA[], $2::JSONB[]) AS t(wh, md)
            ON CONFLICT (wasm_hash) DO UPDATE SET metadata = EXCLUDED.metadata
            "#,
        )
        .bind(&hashes)
        .bind(&metadatas)
        .execute(&mut **db_tx)
        .await?;
    }
    Ok(())
}

/// Pre-insert stub `wasm_interface_metadata` rows for any `wasm_hash`
/// referenced by `staged.contract_rows` but not uploaded in this ledger
/// (task 0153). Mid-stream backfill hits contracts whose WASM was uploaded
/// before the backfill window — the FK
/// `soroban_contracts.wasm_hash -> wasm_interface_metadata.wasm_hash`
/// would otherwise fail. Stubs carry empty metadata; `upsert_wasm_metadata`
/// overwrites them in place once the real upload is observed (ON CONFLICT
/// DO UPDATE), and the empty object is a safe sentinel because WASM bytes
/// are content-addressed by hash.
pub(super) async fn stub_unknown_wasm_interfaces(
    db_tx: &mut Transaction<'_, Postgres>,
    staged: &Staged,
) -> Result<(), HandlerError> {
    let staged_hashes: HashSet<[u8; 32]> = staged.wasm_rows.iter().map(|r| r.wasm_hash).collect();
    let mut seen: HashSet<[u8; 32]> = HashSet::new();
    let mut needed: Vec<Vec<u8>> = Vec::new();
    for row in &staged.contract_rows {
        if let Some(h) = row.wasm_hash
            && !staged_hashes.contains(&h)
            && seen.insert(h)
        {
            needed.push(h.to_vec());
        }
    }
    if needed.is_empty() {
        return Ok(());
    }
    sqlx::query(
        r#"
        INSERT INTO wasm_interface_metadata (wasm_hash, metadata)
        SELECT wh, '{}'::jsonb
          FROM UNNEST($1::BYTEA[]) AS t(wh)
        ON CONFLICT (wasm_hash) DO NOTHING
        "#,
    )
    .bind(&needed)
    .execute(&mut **db_tx)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Task 0118 Phase 2 — back-propagate wasm-spec classification to every
// `soroban_contracts` row sharing a `wasm_hash` touched by this ledger.
// ---------------------------------------------------------------------------

/// UPDATE `soroban_contracts.contract_type` for rows whose `wasm_hash` was
/// classified in this ledger (see `staging::Staged::wasm_classification`).
///
/// Semantics:
///   * Only definitive verdicts (`Nft`, `Fungible`) drive the UPDATE.
///     `Other` carries no information the filter can rely on and would
///     needlessly churn rows.
///   * Rows with `contract_type = Token` are left alone — SACs are
///     authoritative at deploy time (they have no WASM, so a shared
///     `wasm_hash` cannot belong to one) but the guard is defensive.
///   * The UPDATE runs inside the persist tx so the subsequent NFT filter
///     step's SELECT reads the new classification.
///
/// Idempotent on replay: the WHERE `contract_type <> …EXCLUDED…` guard
/// short-circuits no-op writes.
pub(super) async fn reclassify_contracts_from_wasm(
    db_tx: &mut Transaction<'_, Postgres>,
    staged: &Staged,
) -> Result<(), HandlerError> {
    if staged.wasm_classification.is_empty() {
        return Ok(());
    }
    let mut hashes: Vec<Vec<u8>> = Vec::new();
    let mut types: Vec<ContractType> = Vec::new();
    for (hash, &ty) in &staged.wasm_classification {
        if matches!(ty, ContractType::Nft | ContractType::Fungible) {
            hashes.push(hash.to_vec());
            types.push(ty);
        }
    }
    if hashes.is_empty() {
        return Ok(());
    }

    sqlx::query(
        r#"
        UPDATE soroban_contracts sc
           SET contract_type = t.ty
          FROM UNNEST($1::BYTEA[], $2::SMALLINT[]) AS t(wh, ty)
         WHERE sc.wasm_hash = t.wh
           AND sc.contract_type IS DISTINCT FROM 0  -- leave SACs alone
           AND sc.contract_type IS DISTINCT FROM t.ty
        "#,
    )
    .bind(&hashes)
    .bind(&types)
    .execute(&mut **db_tx)
    .await?;
    Ok(())
}

/// Populate the per-worker classification cache from the rows we just
/// upserted. Runs outside the DB and outside the transaction — pure
/// in-memory bookkeeping so a later ledger avoids the SELECT round trip.
///
/// SAC contracts land as `Token`; non-SAC contracts land as whatever
/// classification survived the staging override (`Nft` / `Fungible` if
/// their wasm_hash was observed this ledger, otherwise `Other`, which
/// the cache deliberately drops).
pub(super) fn populate_cache_from_staged(staged: &Staged, cache: &ClassificationCache) {
    cache.extend_definitive(
        staged
            .contract_rows
            .iter()
            .map(|r| (r.contract_id.clone(), r.contract_type)),
    );
}

// ---------------------------------------------------------------------------
// Task 0120 — bridge late-WASM reclassification to the `tokens` table.
// ---------------------------------------------------------------------------

/// Insert a Soroban token row for every `soroban_contracts` row that was
/// promoted to `Fungible` via a WASM upload observed in this ledger,
/// unless such a tokens row already exists.
///
/// Why this step exists:
///
/// `detect_tokens` only emits rows for contracts whose WASM interface is
/// present in the same ledger as the deployment. A two-ledger pattern
/// (contract deployed in ledger N without WASM → WASM uploaded in
/// ledger N+k) leaves `soroban_contracts.contract_type` correct after
/// [`reclassify_contracts_from_wasm`], but no tokens row ever gets
/// created — the deployment row has already been persisted and no longer
/// passes through `detect_tokens`. This step closes that gap by consulting
/// the DB after reclassification.
///
/// Semantics:
///
/// * Runs inside the persist tx, after both
///   [`reclassify_contracts_from_wasm`] and [`upsert_tokens`] have
///   executed earlier in the same transaction. That ordering guarantees
///   (a) `soroban_contracts.contract_type` is authoritative, and (b)
///   any row this ledger's `detect_tokens` already produced is present
///   and won't be duplicated.
/// * Idempotent on replay via `NOT EXISTS` + `ON CONFLICT DO NOTHING`.
/// * Only acts on `Fungible` classifications (tokens side). `Nft` and
///   `Other` are no-ops here — NFTs live in the `nfts` table and `Other`
///   carries no token identity.
pub(super) async fn insert_tokens_from_reclassified_contracts(
    db_tx: &mut Transaction<'_, Postgres>,
    staged: &Staged,
) -> Result<(), HandlerError> {
    // Collect only the Fungible wasm_hashes observed this ledger. NFT and
    // Other verdicts are not token candidates.
    let fungible_hashes: Vec<Vec<u8>> = staged
        .wasm_classification
        .iter()
        .filter(|(_h, ty)| matches!(ty, ContractType::Fungible))
        .map(|(h, _ty)| h.to_vec())
        .collect();

    if fungible_hashes.is_empty() {
        return Ok(());
    }

    sqlx::query(
        r#"
        INSERT INTO tokens (asset_type, contract_id)
        SELECT $1::SMALLINT, sc.id
          FROM soroban_contracts sc
         WHERE sc.wasm_hash = ANY($2::BYTEA[])
           AND sc.contract_type = $3::SMALLINT
           AND NOT EXISTS (
                 SELECT 1 FROM tokens t
                  WHERE t.contract_id = sc.id
                    AND t.asset_type IN (2, 3)  -- sac, soroban
               )
        ON CONFLICT (contract_id)
          WHERE asset_type IN (2, 3)
          DO NOTHING
        "#,
    )
    .bind(TokenAssetType::Soroban)
    .bind(&fungible_hashes)
    .bind(ContractType::Fungible)
    .execute(&mut **db_tx)
    .await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// 3. soroban_contracts — upsert, returning StrKey → surrogate id map (ADR 0030)
// ---------------------------------------------------------------------------

/// Upsert every contract StrKey referenced in this ledger and return the
/// StrKey → `soroban_contracts.id` map. Mirrors `upsert_accounts`.
///
/// Two passes:
///   1. Rich rows from `staged.contract_rows` — carry deployment/WASM metadata.
///      `ON CONFLICT DO UPDATE` rewrites no-op columns so `RETURNING` fires
///      on both insert and replay paths.
///   2. Referenced-only contract StrKeys from ops/events/invocations/
///      tokens/nfts that weren't deployed this ledger. Bare-row upsert with
///      the same no-op `DO UPDATE` trick so `RETURNING` populates the map.
pub(super) async fn upsert_contracts_returning_id(
    db_tx: &mut Transaction<'_, Postgres>,
    staged: &Staged,
    account_ids: &HashMap<String, i64>,
) -> Result<HashMap<String, i64>, HandlerError> {
    let mut out: HashMap<String, i64> = HashMap::new();

    // Pass 1 — rich rows with metadata.
    for chunk in staged.contract_rows.chunks(CHUNK_SIZE) {
        let mut contract_ids: Vec<String> = Vec::with_capacity(chunk.len());
        let mut wasm_hashes: Vec<Option<Vec<u8>>> = Vec::with_capacity(chunk.len());
        let mut uploaded: Vec<Option<i64>> = Vec::with_capacity(chunk.len());
        let mut deployers: Vec<Option<i64>> = Vec::with_capacity(chunk.len());
        let mut deployed: Vec<Option<i64>> = Vec::with_capacity(chunk.len());
        // ADR 0031: contract_type is SMALLINT (Rust ContractType enum).
        let mut types: Vec<Option<ContractType>> = Vec::with_capacity(chunk.len());
        let mut sacs: Vec<bool> = Vec::with_capacity(chunk.len());
        let mut metadatas: Vec<Option<Value>> = Vec::with_capacity(chunk.len());

        for r in chunk {
            contract_ids.push(r.contract_id.clone());
            wasm_hashes.push(r.wasm_hash.map(|h| h.to_vec()));
            uploaded.push(r.wasm_uploaded_at_ledger);
            deployers.push(
                r.deployer_str_key
                    .as_ref()
                    .and_then(|k| account_ids.get(k).copied()),
            );
            deployed.push(r.deployed_at_ledger);
            types.push(Some(r.contract_type));
            sacs.push(r.is_sac);
            metadatas.push(r.metadata.clone());
        }

        let rows: Vec<(i64, String)> = sqlx::query_as(
            r#"
            INSERT INTO soroban_contracts (
                contract_id, wasm_hash, wasm_uploaded_at_ledger, deployer_id,
                deployed_at_ledger, contract_type, is_sac, metadata
            )
            SELECT * FROM UNNEST(
                $1::VARCHAR[], $2::BYTEA[], $3::BIGINT[], $4::BIGINT[],
                $5::BIGINT[], $6::SMALLINT[], $7::BOOL[], $8::JSONB[]
            )
            ON CONFLICT (contract_id) DO UPDATE SET
                wasm_hash = COALESCE(EXCLUDED.wasm_hash, soroban_contracts.wasm_hash),
                deployer_id = COALESCE(EXCLUDED.deployer_id, soroban_contracts.deployer_id),
                deployed_at_ledger = COALESCE(EXCLUDED.deployed_at_ledger, soroban_contracts.deployed_at_ledger),
                contract_type = COALESCE(EXCLUDED.contract_type, soroban_contracts.contract_type),
                is_sac = soroban_contracts.is_sac OR EXCLUDED.is_sac,
                metadata = COALESCE(EXCLUDED.metadata, soroban_contracts.metadata)
            RETURNING id, contract_id
            "#,
        )
        .bind(&contract_ids)
        .bind(&wasm_hashes)
        .bind(&uploaded)
        .bind(&deployers)
        .bind(&deployed)
        .bind(&types)
        .bind(&sacs)
        .bind(&metadatas)
        .fetch_all(&mut **db_tx)
        .await?;

        for (id, key) in rows {
            out.insert(key, id);
        }
    }

    // Pass 2 — referenced-only StrKeys (not deployed this ledger).
    let mut extras: Vec<String> = Vec::new();
    let mut consider = |cid: Option<&String>| {
        if let Some(c) = cid
            && !c.is_empty()
            && !out.contains_key(c.as_str())
            && !extras.iter().any(|e| e == c)
        {
            extras.push(c.clone());
        }
    };
    for row in &staged.op_rows {
        consider(row.contract_id.as_ref());
    }
    for row in &staged.event_rows {
        consider(row.contract_id.as_ref());
    }
    for row in &staged.inv_rows {
        consider(row.contract_id.as_ref());
    }
    for row in &staged.token_rows {
        consider(row.contract_id.as_ref());
    }
    for row in &staged.nft_rows {
        consider(Some(&row.contract_id));
    }
    if extras.is_empty() {
        return Ok(out);
    }

    for chunk in extras.chunks(CHUNK_SIZE) {
        let cids: Vec<String> = chunk.to_vec();
        // No-op `DO UPDATE SET contract_id = EXCLUDED.contract_id` ensures
        // `RETURNING` fires on both insert and replay (ON CONFLICT DO NOTHING
        // suppresses RETURNING for the conflicting row).
        let rows: Vec<(i64, String)> = sqlx::query_as(
            r#"
            INSERT INTO soroban_contracts (contract_id, is_sac)
            SELECT cid, false
              FROM UNNEST($1::VARCHAR[]) AS t(cid)
            ON CONFLICT (contract_id) DO UPDATE SET contract_id = EXCLUDED.contract_id
            RETURNING id, contract_id
            "#,
        )
        .bind(&cids)
        .fetch_all(&mut **db_tx)
        .await?;

        for (id, key) in rows {
            out.insert(key, id);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// 4. ledgers — idempotent insert
// ---------------------------------------------------------------------------

pub(super) async fn insert_ledger(
    db_tx: &mut Transaction<'_, Postgres>,
    staged: &Staged,
) -> Result<(), HandlerError> {
    sqlx::query(
        r#"
        INSERT INTO ledgers (sequence, hash, closed_at, protocol_version, transaction_count, base_fee)
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (sequence) DO NOTHING
        "#,
    )
    .bind(staged.ledger_sequence_i64)
    .bind(staged.ledger_hash.as_slice())
    .bind(staged.ledger_closed_at)
    .bind(staged.ledger_protocol_version)
    .bind(staged.ledger_transaction_count)
    .bind(staged.ledger_base_fee)
    .execute(&mut **db_tx)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// 5. transactions — insert RETURNING id, building hash → id map
// ---------------------------------------------------------------------------

pub(super) async fn insert_transactions(
    db_tx: &mut Transaction<'_, Postgres>,
    staged: &Staged,
    account_ids: &HashMap<String, i64>,
) -> Result<HashMap<String, i64>, HandlerError> {
    let mut out: HashMap<String, i64> = HashMap::with_capacity(staged.tx_rows.len());
    if staged.tx_rows.is_empty() {
        return Ok(out);
    }

    for chunk in staged.tx_rows.chunks(CHUNK_SIZE) {
        let mut hashes: Vec<Vec<u8>> = Vec::with_capacity(chunk.len());
        let mut ledger_seqs: Vec<i64> = Vec::with_capacity(chunk.len());
        let mut app_orders: Vec<i16> = Vec::with_capacity(chunk.len());
        let mut source_ids: Vec<i64> = Vec::with_capacity(chunk.len());
        let mut fees: Vec<i64> = Vec::with_capacity(chunk.len());
        let mut inner_hashes: Vec<Option<Vec<u8>>> = Vec::with_capacity(chunk.len());
        let mut successes: Vec<bool> = Vec::with_capacity(chunk.len());
        let mut op_counts: Vec<i16> = Vec::with_capacity(chunk.len());
        let mut has_sorobans: Vec<bool> = Vec::with_capacity(chunk.len());
        let mut parse_errors: Vec<bool> = Vec::with_capacity(chunk.len());
        let mut created_ats: Vec<DateTime<Utc>> = Vec::with_capacity(chunk.len());

        for r in chunk {
            hashes.push(r.hash.to_vec());
            ledger_seqs.push(r.ledger_sequence);
            app_orders.push(r.application_order);
            source_ids.push(resolve_id(
                account_ids,
                &r.source_str_key,
                "transactions.source",
            )?);
            fees.push(r.fee_charged);
            inner_hashes.push(r.inner_tx_hash.map(|h| h.to_vec()));
            successes.push(r.successful);
            op_counts.push(r.operation_count);
            has_sorobans.push(r.has_soroban);
            parse_errors.push(r.parse_error);
            created_ats.push(r.created_at);
        }

        // ON CONFLICT targets `uq_transactions_hash_created_at` (migration
        // 20260421000000). Partitioned UNIQUE must include the partition key
        // so the constraint is `(hash, created_at)`; `created_at` is derived
        // from ledger close time, so it matches on replay.
        //
        // The `DO UPDATE SET hash = EXCLUDED.hash` form is a deliberate no-op
        // that still fires RETURNING — we need the id on both insert and
        // replay paths to populate `tx_ids`.
        let rows: Vec<(i64, Vec<u8>)> = sqlx::query_as(
            r#"
            INSERT INTO transactions (
                hash, ledger_sequence, application_order, source_id, fee_charged,
                inner_tx_hash, successful, operation_count, has_soroban, parse_error, created_at
            )
            SELECT * FROM UNNEST(
                $1::BYTEA[], $2::BIGINT[], $3::SMALLINT[], $4::BIGINT[], $5::BIGINT[],
                $6::BYTEA[], $7::BOOL[], $8::SMALLINT[], $9::BOOL[], $10::BOOL[], $11::TIMESTAMPTZ[]
            )
            ON CONFLICT ON CONSTRAINT uq_transactions_hash_created_at
            DO UPDATE SET hash = EXCLUDED.hash
            RETURNING id, hash
            "#,
        )
        .bind(&hashes)
        .bind(&ledger_seqs)
        .bind(&app_orders)
        .bind(&source_ids)
        .bind(&fees)
        .bind(&inner_hashes)
        .bind(&successes)
        .bind(&op_counts)
        .bind(&has_sorobans)
        .bind(&parse_errors)
        .bind(&created_ats)
        .fetch_all(&mut **db_tx)
        .await?;

        let expected_len = hashes.len();
        if rows.len() != expected_len {
            return Err(HandlerError::Staging(format!(
                "transactions RETURNING row count mismatch: got {}, expected {}",
                rows.len(),
                expected_len
            )));
        }

        for (id, hash_bytes) in rows {
            out.insert(hex::encode(hash_bytes), id);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// 6. transaction_hash_index — idempotent insert
// ---------------------------------------------------------------------------

pub(super) async fn insert_hash_index(
    db_tx: &mut Transaction<'_, Postgres>,
    staged: &Staged,
) -> Result<(), HandlerError> {
    if staged.tx_rows.is_empty() {
        return Ok(());
    }
    for chunk in staged.tx_rows.chunks(CHUNK_SIZE) {
        let hashes: Vec<Vec<u8>> = chunk.iter().map(|r: &TxRow| r.hash.to_vec()).collect();
        let seqs: Vec<i64> = chunk.iter().map(|r| r.ledger_sequence).collect();
        let created_ats: Vec<DateTime<Utc>> = chunk.iter().map(|r| r.created_at).collect();
        sqlx::query(
            r#"
            INSERT INTO transaction_hash_index (hash, ledger_sequence, created_at)
            SELECT * FROM UNNEST($1::BYTEA[], $2::BIGINT[], $3::TIMESTAMPTZ[])
            ON CONFLICT (hash) DO NOTHING
            "#,
        )
        .bind(&hashes)
        .bind(&seqs)
        .bind(&created_ats)
        .execute(&mut **db_tx)
        .await?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 7. transaction_participants — idempotent insert
// ---------------------------------------------------------------------------

pub(super) async fn insert_participants(
    db_tx: &mut Transaction<'_, Postgres>,
    staged: &Staged,
    account_ids: &HashMap<String, i64>,
    tx_ids: &HashMap<String, i64>,
) -> Result<(), HandlerError> {
    if staged.participant_rows.is_empty() {
        return Ok(());
    }
    for chunk in staged.participant_rows.chunks(CHUNK_SIZE) {
        let mut tx_id_vec: Vec<i64> = Vec::with_capacity(chunk.len());
        let mut acct_id_vec: Vec<i64> = Vec::with_capacity(chunk.len());
        let mut created_vec: Vec<DateTime<Utc>> = Vec::with_capacity(chunk.len());

        for r in chunk {
            let Some(tx_id) = tx_ids.get(&r.tx_hash_hex).copied() else {
                continue;
            };
            tx_id_vec.push(tx_id);
            acct_id_vec.push(resolve_id(
                account_ids,
                &r.account_str_key,
                "participants.account_id",
            )?);
            created_vec.push(r.created_at);
        }

        if tx_id_vec.is_empty() {
            continue;
        }

        sqlx::query(
            r#"
            INSERT INTO transaction_participants (transaction_id, account_id, created_at)
            SELECT * FROM UNNEST($1::BIGINT[], $2::BIGINT[], $3::TIMESTAMPTZ[])
            ON CONFLICT (account_id, created_at, transaction_id) DO NOTHING
            "#,
        )
        .bind(&tx_id_vec)
        .bind(&acct_id_vec)
        .bind(&created_vec)
        .execute(&mut **db_tx)
        .await?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 8. operations — insert typed-cols
// ---------------------------------------------------------------------------

pub(super) async fn insert_operations(
    db_tx: &mut Transaction<'_, Postgres>,
    staged: &Staged,
    account_ids: &HashMap<String, i64>,
    contract_ids: &HashMap<String, i64>,
    tx_ids: &HashMap<String, i64>,
) -> Result<(), HandlerError> {
    if staged.op_rows.is_empty() {
        return Ok(());
    }
    for chunk in staged.op_rows.chunks(CHUNK_SIZE) {
        let mut tx_id_vec: Vec<i64> = Vec::with_capacity(chunk.len());
        let mut app_order_vec: Vec<i16> = Vec::with_capacity(chunk.len());
        // ADR 0031: operations.type is SMALLINT (Rust OperationType enum).
        let mut op_type_vec: Vec<OperationType> = Vec::with_capacity(chunk.len());
        let mut source_id_vec: Vec<Option<i64>> = Vec::with_capacity(chunk.len());
        let mut dest_id_vec: Vec<Option<i64>> = Vec::with_capacity(chunk.len());
        let mut contract_vec: Vec<Option<i64>> = Vec::with_capacity(chunk.len());
        let mut asset_code_vec: Vec<Option<String>> = Vec::with_capacity(chunk.len());
        let mut asset_issuer_vec: Vec<Option<i64>> = Vec::with_capacity(chunk.len());
        let mut pool_id_vec: Vec<Option<Vec<u8>>> = Vec::with_capacity(chunk.len());
        let mut transfer_amt_vec: Vec<Option<String>> = Vec::with_capacity(chunk.len());
        let mut ledger_seq_vec: Vec<i64> = Vec::with_capacity(chunk.len());
        let mut created_at_vec: Vec<DateTime<Utc>> = Vec::with_capacity(chunk.len());

        for r in chunk {
            let Some(tx_id) = tx_ids.get(&r.tx_hash_hex).copied() else {
                continue;
            };
            tx_id_vec.push(tx_id);
            app_order_vec.push(r.application_order);
            op_type_vec.push(r.op_type);
            source_id_vec.push(resolve_opt_id(
                account_ids,
                r.source_str_key.as_deref(),
                "op.source",
            )?);
            dest_id_vec.push(resolve_opt_id(
                account_ids,
                r.destination_str_key.as_deref(),
                "op.destination",
            )?);
            contract_vec.push(resolve_contract_opt_id(
                contract_ids,
                r.contract_id.as_deref(),
                "op.contract",
            )?);
            asset_code_vec.push(r.asset_code.clone());
            asset_issuer_vec.push(resolve_opt_id(
                account_ids,
                r.asset_issuer_str_key.as_deref(),
                "op.asset_issuer",
            )?);
            pool_id_vec.push(r.pool_id.map(|h| h.to_vec()));
            transfer_amt_vec.push(r.transfer_amount.clone());
            ledger_seq_vec.push(r.ledger_sequence);
            created_at_vec.push(r.created_at);
        }

        if tx_id_vec.is_empty() {
            continue;
        }

        // `operations.pool_id` → `liquidity_pools.pool_id` FK must hold, but
        // a backfill starting mid-stream can see DEPOSIT/WITHDRAW ops
        // targeting pools created in un-indexed earlier ledgers. Nullify
        // pool_id when the referenced pool is not present; the op row stays,
        // only the FK link turns NULL for historical references.
        sqlx::query(
            r#"
            INSERT INTO operations (
                transaction_id, application_order, type, source_id, destination_id,
                contract_id, asset_code, asset_issuer_id, pool_id, transfer_amount,
                ledger_sequence, created_at
            )
            SELECT
                t.tx_id, t.app_order, t.op_type, t.source_id, t.dest_id,
                t.contract_id, t.asset_code, t.asset_issuer_id,
                CASE
                    WHEN t.pool_id IS NULL THEN NULL
                    WHEN EXISTS (SELECT 1 FROM liquidity_pools lp WHERE lp.pool_id = t.pool_id) THEN t.pool_id
                    ELSE NULL
                END,
                CASE WHEN t.txt IS NULL THEN NULL ELSE t.txt::NUMERIC(28,7) END,
                t.ledger_sequence, t.created_at
              FROM UNNEST(
                $1::BIGINT[], $2::SMALLINT[], $3::SMALLINT[], $4::BIGINT[], $5::BIGINT[],
                $6::BIGINT[], $7::VARCHAR[], $8::BIGINT[], $9::BYTEA[], $10::TEXT[],
                $11::BIGINT[], $12::TIMESTAMPTZ[]
              )
                AS t(tx_id, app_order, op_type, source_id, dest_id,
                     contract_id, asset_code, asset_issuer_id, pool_id, txt,
                     ledger_sequence, created_at)
            ON CONFLICT ON CONSTRAINT uq_operations_tx_order DO NOTHING
            "#,
        )
        .bind(&tx_id_vec)
        .bind(&app_order_vec)
        .bind(&op_type_vec)
        .bind(&source_id_vec)
        .bind(&dest_id_vec)
        .bind(&contract_vec)
        .bind(&asset_code_vec)
        .bind(&asset_issuer_vec)
        .bind(&pool_id_vec)
        .bind(&transfer_amt_vec)
        .bind(&ledger_seq_vec)
        .bind(&created_at_vec)
        .execute(&mut **db_tx)
        .await?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 9. soroban_events_appearances — ADR 0033 appearance index
// ---------------------------------------------------------------------------

/// Aggregate staged contract events into `(contract, tx, ledger)` appearance
/// rows and insert them. The `amount` column stores the number of non-
/// diagnostic contract events folded into the trio; all parsed event detail
/// (type, topics, data, per-event index, transfer triple) is re-materialised
/// at read time from the public Stellar archive via
/// `xdr_parser::extract_events`.
///
/// Events without a resolved `contract_id` (system events with no emitter
/// or contracts the indexer hasn't seen yet) are skipped — the appearance
/// index is contract-scoped by construction.
///
/// Replay-safe: the composite PK covers the natural key, so a re-ingest of
/// the same ledger produces zero duplicate rows via `ON CONFLICT DO NOTHING`.
pub(super) async fn insert_events(
    db_tx: &mut Transaction<'_, Postgres>,
    staged: &Staged,
    contract_ids: &HashMap<String, i64>,
    tx_ids: &HashMap<String, i64>,
) -> Result<(), HandlerError> {
    if staged.event_rows.is_empty() {
        return Ok(());
    }

    // Key: (contract_id, transaction_id, ledger_sequence, created_at).
    //
    // `upsert_contracts_returning_id` seeds `contract_ids` from every
    // `contract_id` referenced in `staged.event_rows`, so a present
    // `contract_id` here MUST resolve — a miss is an invariant violation
    // (hard error, not silent skip). A missing `tx_id` still skips
    // silently per repo convention (tx may be dropped at staging for
    // parse errors that don't abort the whole ledger).
    let mut agg: HashMap<(i64, i64, i64, DateTime<Utc>), i64> = HashMap::new();
    for r in &staged.event_rows {
        let Some(contract_key) = r.contract_id.as_deref() else {
            continue;
        };
        let contract_id = resolve_contract_id(contract_ids, contract_key, "event.contract")?;
        let Some(&tx_id) = tx_ids.get(&r.tx_hash_hex) else {
            continue;
        };
        *agg.entry((contract_id, tx_id, r.ledger_sequence, r.created_at))
            .or_insert(0) += 1;
    }

    if agg.is_empty() {
        return Ok(());
    }

    let rows: Vec<_> = agg.into_iter().collect();
    for chunk in rows.chunks(CHUNK_SIZE) {
        let mut contract_vec: Vec<i64> = Vec::with_capacity(chunk.len());
        let mut tx_id_vec: Vec<i64> = Vec::with_capacity(chunk.len());
        let mut ls_vec: Vec<i64> = Vec::with_capacity(chunk.len());
        let mut amount_vec: Vec<i64> = Vec::with_capacity(chunk.len());
        let mut ca_vec: Vec<DateTime<Utc>> = Vec::with_capacity(chunk.len());

        for &((contract_id, tx_id, ledger_sequence, created_at), amount) in chunk {
            contract_vec.push(contract_id);
            tx_id_vec.push(tx_id);
            ls_vec.push(ledger_sequence);
            amount_vec.push(amount);
            ca_vec.push(created_at);
        }

        sqlx::query(
            r#"
            INSERT INTO soroban_events_appearances (
                contract_id, transaction_id, ledger_sequence, amount, created_at
            )
            SELECT * FROM UNNEST(
                $1::BIGINT[], $2::BIGINT[], $3::BIGINT[], $4::BIGINT[], $5::TIMESTAMPTZ[]
            )
            ON CONFLICT (contract_id, transaction_id, ledger_sequence, created_at) DO NOTHING
            "#,
        )
        .bind(&contract_vec)
        .bind(&tx_id_vec)
        .bind(&ls_vec)
        .bind(&amount_vec)
        .bind(&ca_vec)
        .execute(&mut **db_tx)
        .await?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 10. soroban_invocations — insert slim cols
// ---------------------------------------------------------------------------

pub(super) async fn insert_invocations(
    db_tx: &mut Transaction<'_, Postgres>,
    staged: &Staged,
    account_ids: &HashMap<String, i64>,
    contract_ids: &HashMap<String, i64>,
    tx_ids: &HashMap<String, i64>,
) -> Result<(), HandlerError> {
    if staged.inv_rows.is_empty() {
        return Ok(());
    }
    for chunk in staged.inv_rows.chunks(CHUNK_SIZE) {
        let mut tx_id_vec: Vec<i64> = Vec::with_capacity(chunk.len());
        let mut contract_vec: Vec<Option<i64>> = Vec::with_capacity(chunk.len());
        let mut caller_vec: Vec<Option<i64>> = Vec::with_capacity(chunk.len());
        let mut fname_vec: Vec<String> = Vec::with_capacity(chunk.len());
        let mut success_vec: Vec<bool> = Vec::with_capacity(chunk.len());
        let mut index_vec: Vec<i16> = Vec::with_capacity(chunk.len());
        let mut ls_vec: Vec<i64> = Vec::with_capacity(chunk.len());
        let mut ca_vec: Vec<DateTime<Utc>> = Vec::with_capacity(chunk.len());

        for r in chunk {
            let Some(tx_id) = tx_ids.get(&r.tx_hash_hex).copied() else {
                continue;
            };
            tx_id_vec.push(tx_id);
            contract_vec.push(resolve_contract_opt_id(
                contract_ids,
                r.contract_id.as_deref(),
                "invocation.contract",
            )?);
            caller_vec.push(resolve_opt_id(
                account_ids,
                r.caller_str_key.as_deref(),
                "invocation.caller",
            )?);
            fname_vec.push(r.function_name.clone());
            success_vec.push(r.successful);
            index_vec.push(r.invocation_index);
            ls_vec.push(r.ledger_sequence);
            ca_vec.push(r.created_at);
        }

        if tx_id_vec.is_empty() {
            continue;
        }

        sqlx::query(
            r#"
            INSERT INTO soroban_invocations (
                transaction_id, contract_id, caller_id, function_name, successful,
                invocation_index, ledger_sequence, created_at
            )
            SELECT * FROM UNNEST(
                $1::BIGINT[], $2::BIGINT[], $3::BIGINT[], $4::VARCHAR[], $5::BOOL[],
                $6::SMALLINT[], $7::BIGINT[], $8::TIMESTAMPTZ[]
            )
            ON CONFLICT ON CONSTRAINT uq_soroban_invocations_tx_index DO NOTHING
            "#,
        )
        .bind(&tx_id_vec)
        .bind(&contract_vec)
        .bind(&caller_vec)
        .bind(&fname_vec)
        .bind(&success_vec)
        .bind(&index_vec)
        .bind(&ls_vec)
        .bind(&ca_vec)
        .execute(&mut **db_tx)
        .await?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 11. tokens — upsert honouring ck_tokens_identity
// ---------------------------------------------------------------------------

pub(super) async fn upsert_tokens(
    db_tx: &mut Transaction<'_, Postgres>,
    staged: &Staged,
    account_ids: &HashMap<String, i64>,
    contract_ids: &HashMap<String, i64>,
) -> Result<(), HandlerError> {
    if staged.token_rows.is_empty() {
        return Ok(());
    }
    // Separate paths per identity class — each has its own partial UNIQUE.
    let mut native: Vec<&TokenRow> = Vec::new();
    let mut classic: Vec<&TokenRow> = Vec::new();
    let mut sac: Vec<&TokenRow> = Vec::new();
    let mut soroban: Vec<&TokenRow> = Vec::new();

    for t in &staged.token_rows {
        match t.asset_type {
            TokenAssetType::Native => native.push(t),
            TokenAssetType::Classic => classic.push(t),
            TokenAssetType::Sac => sac.push(t),
            TokenAssetType::Soroban => soroban.push(t),
        }
    }

    upsert_tokens_native(db_tx, &native).await?;
    upsert_tokens_classic_like(
        db_tx,
        &classic,
        TokenAssetType::Classic,
        account_ids,
        contract_ids,
    )
    .await?;
    upsert_tokens_classic_like(db_tx, &sac, TokenAssetType::Sac, account_ids, contract_ids).await?;
    upsert_tokens_soroban(db_tx, &soroban, contract_ids).await?;

    Ok(())
}

async fn upsert_tokens_native(
    db_tx: &mut Transaction<'_, Postgres>,
    rows: &[&TokenRow],
) -> Result<(), HandlerError> {
    if rows.is_empty() {
        return Ok(());
    }
    // Only one native token can exist (uidx_tokens_native). De-dup here so the
    // INSERT binds exactly one row.
    let (name, total_supply, holder_count) = rows
        .first()
        .map(|t| (t.name.clone(), t.total_supply.clone(), t.holder_count))
        .unwrap_or((None, None, None));
    // ADR 0031: tokens.asset_type is SMALLINT — bind the enum, don't inline a literal.
    sqlx::query(
        r#"
        INSERT INTO tokens (asset_type, name, total_supply, holder_count)
        SELECT $1, $2, CASE WHEN $3 IS NULL THEN NULL ELSE $3::NUMERIC(28,7) END, $4
        WHERE NOT EXISTS (SELECT 1 FROM tokens WHERE asset_type = $1)
        "#,
    )
    .bind(TokenAssetType::Native)
    .bind(name)
    .bind(total_supply)
    .bind(holder_count)
    .execute(&mut **db_tx)
    .await?;
    Ok(())
}

async fn upsert_tokens_classic_like(
    db_tx: &mut Transaction<'_, Postgres>,
    rows: &[&TokenRow],
    asset_type: TokenAssetType,
    account_ids: &HashMap<String, i64>,
    contract_ids: &HashMap<String, i64>,
) -> Result<(), HandlerError> {
    debug_assert!(
        matches!(asset_type, TokenAssetType::Classic | TokenAssetType::Sac),
        "upsert_tokens_classic_like only handles classic/sac; got {asset_type:?}"
    );
    if rows.is_empty() {
        return Ok(());
    }
    for chunk in rows.chunks(CHUNK_SIZE) {
        let mut codes: Vec<String> = Vec::with_capacity(chunk.len());
        let mut issuers: Vec<i64> = Vec::with_capacity(chunk.len());
        let mut contracts: Vec<Option<i64>> = Vec::with_capacity(chunk.len());
        let mut names: Vec<Option<String>> = Vec::with_capacity(chunk.len());
        let mut supplies: Vec<Option<String>> = Vec::with_capacity(chunk.len());
        let mut holders: Vec<Option<i32>> = Vec::with_capacity(chunk.len());

        for r in chunk {
            let Some(code) = r.asset_code.as_ref() else {
                continue;
            };
            let Some(issuer_key) = r.issuer_str_key.as_ref() else {
                continue;
            };
            let issuer_id = resolve_id(account_ids, issuer_key, "token.issuer")?;
            codes.push(code.clone());
            issuers.push(issuer_id);
            contracts.push(resolve_contract_opt_id(
                contract_ids,
                r.contract_id.as_deref(),
                "token.contract",
            )?);
            names.push(r.name.clone());
            supplies.push(r.total_supply.clone());
            holders.push(r.holder_count);
        }
        if codes.is_empty() {
            continue;
        }

        // ADR 0031: bind asset_type as SMALLINT enum; partial UNIQUE index on
        // `tokens (asset_code, issuer_id) WHERE asset_type IN (1, 2)` (classic, sac)
        // matches numeric ordinals — see migration 0005.
        sqlx::query(
            r#"
            INSERT INTO tokens (asset_type, asset_code, issuer_id, contract_id, name, total_supply, holder_count)
            SELECT $1, code, issuer_id, contract_id, name,
                   CASE WHEN supply IS NULL THEN NULL ELSE supply::NUMERIC(28,7) END, holder_count
              FROM UNNEST($2::VARCHAR[], $3::BIGINT[], $4::BIGINT[], $5::VARCHAR[], $6::TEXT[], $7::INTEGER[])
                AS t(code, issuer_id, contract_id, name, supply, holder_count)
            ON CONFLICT (asset_code, issuer_id)
              WHERE asset_type IN (1, 2)  -- classic, sac
              DO UPDATE SET
                contract_id = COALESCE(EXCLUDED.contract_id, tokens.contract_id),
                name = COALESCE(EXCLUDED.name, tokens.name),
                total_supply = COALESCE(EXCLUDED.total_supply, tokens.total_supply),
                holder_count = COALESCE(EXCLUDED.holder_count, tokens.holder_count)
            "#,
        )
        .bind(asset_type)
        .bind(&codes)
        .bind(&issuers)
        .bind(&contracts)
        .bind(&names)
        .bind(&supplies)
        .bind(&holders)
        .execute(&mut **db_tx)
        .await?;
    }
    Ok(())
}

async fn upsert_tokens_soroban(
    db_tx: &mut Transaction<'_, Postgres>,
    rows: &[&TokenRow],
    contract_ids: &HashMap<String, i64>,
) -> Result<(), HandlerError> {
    if rows.is_empty() {
        return Ok(());
    }
    for chunk in rows.chunks(CHUNK_SIZE) {
        let mut contracts: Vec<i64> = Vec::with_capacity(chunk.len());
        let mut names: Vec<Option<String>> = Vec::with_capacity(chunk.len());
        let mut supplies: Vec<Option<String>> = Vec::with_capacity(chunk.len());
        let mut holders: Vec<Option<i32>> = Vec::with_capacity(chunk.len());

        for r in chunk {
            let Some(cid) = r.contract_id.as_ref() else {
                continue;
            };
            contracts.push(resolve_contract_id(
                contract_ids,
                cid,
                "token.soroban.contract",
            )?);
            names.push(r.name.clone());
            supplies.push(r.total_supply.clone());
            holders.push(r.holder_count);
        }
        if contracts.is_empty() {
            continue;
        }
        // ADR 0031: partial UNIQUE on `tokens (contract_id) WHERE asset_type IN (2, 3)` (sac, soroban).
        sqlx::query(
            r#"
            INSERT INTO tokens (asset_type, contract_id, name, total_supply, holder_count)
            SELECT $1, contract_id, name,
                   CASE WHEN supply IS NULL THEN NULL ELSE supply::NUMERIC(28,7) END, holder_count
              FROM UNNEST($2::BIGINT[], $3::TEXT[], $4::TEXT[], $5::INTEGER[])
                AS t(contract_id, name, supply, holder_count)
            ON CONFLICT (contract_id)
              WHERE asset_type IN (2, 3)  -- sac, soroban
              DO UPDATE SET
                name = COALESCE(EXCLUDED.name, tokens.name),
                total_supply = COALESCE(EXCLUDED.total_supply, tokens.total_supply),
                holder_count = COALESCE(EXCLUDED.holder_count, tokens.holder_count)
            "#,
        )
        .bind(TokenAssetType::Soroban)
        .bind(&contracts)
        .bind(&names)
        .bind(&supplies)
        .bind(&holders)
        .execute(&mut **db_tx)
        .await?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 12. nfts + nft_ownership
// ---------------------------------------------------------------------------

/// Task 0118 Phase 2 — resolve every NFT-candidate contract's
/// classification and decide which `nft_rows` / `nft_ownership_rows`
/// survive the filter. Returns index vectors into the staged slices.
///
/// Flow:
///   1. Collect distinct contract_ids referenced by either slice.
///   2. Read the per-worker cache; anything it doesn't know needs a DB lookup.
///   3. Batch SELECT the misses from `soroban_contracts`.
///   4. Populate the cache with definitive non-NULL verdicts; NULL/invalid
///      rows stay uncached and therefore fall through as "keep" (same as
///      an `Other` verdict, cleaned up by Phase 3 SQL).
///   5. Take one cache snapshot for the candidate set so the per-row
///      filter loop is lock-free.
///   6. Decide insert vs skip per-row:
///      * `Nft`     → insert.
///      * `Other`   → insert (temporary false positive; Phase 3 SQL
///        cleans up once backfill has observed every WASM).
///      * `Token` / `Fungible` → skip.
async fn resolve_nft_filter(
    db_tx: &mut Transaction<'_, Postgres>,
    staged: &Staged,
    cache: &ClassificationCache,
) -> Result<(Vec<usize>, Vec<usize>), HandlerError> {
    let mut candidate_ids: HashSet<&str> = HashSet::new();
    for r in &staged.nft_rows {
        candidate_ids.insert(r.contract_id.as_str());
    }
    for r in &staged.nft_ownership_rows {
        candidate_ids.insert(r.contract_id.as_str());
    }

    if !candidate_ids.is_empty() {
        let misses = cache.missing(candidate_ids.iter().copied());
        if !misses.is_empty() {
            let param: Vec<String> = misses.iter().map(|s| (*s).to_string()).collect();
            let rows: Vec<(String, Option<i16>)> = sqlx::query_as(
                r#"
                SELECT contract_id, contract_type
                  FROM soroban_contracts
                 WHERE contract_id = ANY($1::VARCHAR[])
                "#,
            )
            .bind(&param)
            .fetch_all(&mut **db_tx)
            .await?;
            let fetched: Vec<(String, ContractType)> = rows
                .into_iter()
                .filter_map(|(id, ty)| {
                    ty.and_then(|v| ContractType::try_from(v).ok())
                        .map(|v| (id, v))
                })
                .collect();
            cache.extend_definitive(fetched);
        }
    }

    // One lock round-trip for the whole ledger's candidate set. The per-row
    // filter below then consults the local HashMap without ever touching
    // the shared mutex.
    let snapshot = cache.snapshot_for(candidate_ids.iter().copied());

    let keep = |id: &str| -> bool {
        match snapshot.get(id) {
            Some(ContractType::Token) | Some(ContractType::Fungible) => false,
            // Nft / Other / uncached → insert. Uncached covers NULL DB rows
            // and `Other` verdicts we deliberately don't cache.
            _ => true,
        }
    };

    let nft_indices: Vec<usize> = staged
        .nft_rows
        .iter()
        .enumerate()
        .filter_map(|(i, r)| keep(r.contract_id.as_str()).then_some(i))
        .collect();
    let ownership_indices: Vec<usize> = staged
        .nft_ownership_rows
        .iter()
        .enumerate()
        .filter_map(|(i, r)| keep(r.contract_id.as_str()).then_some(i))
        .collect();
    Ok((nft_indices, ownership_indices))
}

pub(super) async fn upsert_nfts_and_ownership(
    db_tx: &mut Transaction<'_, Postgres>,
    staged: &Staged,
    account_ids: &HashMap<String, i64>,
    contract_ids: &HashMap<String, i64>,
    tx_ids: &HashMap<String, i64>,
    classification_cache: &ClassificationCache,
) -> Result<(), HandlerError> {
    // Task 0118 Phase 2 — classify every contract referenced by an
    // NFT-candidate row, dropping rows whose contract is a known
    // `Fungible` or `Token` (SAC). `Other` rows are preserved (inserted
    // temporarily; the Phase 3 cleanup SQL sweeps them once a backfill
    // has observed every WASM upload).
    let (nft_indices, ownership_indices) =
        resolve_nft_filter(db_tx, staged, classification_cache).await?;

    // 12a. nfts (watermark-guarded on current_owner_ledger)
    //
    // Iterate the surviving index vector directly — avoids the
    // `Vec<&NftRow>` intermediate allocation proportional to the
    // survivor count.
    if !nft_indices.is_empty() {
        for idx_chunk in nft_indices.chunks(CHUNK_SIZE) {
            let mut contracts: Vec<i64> = Vec::with_capacity(idx_chunk.len());
            let mut token_ids: Vec<String> = Vec::with_capacity(idx_chunk.len());
            let mut collections: Vec<Option<String>> = Vec::with_capacity(idx_chunk.len());
            let mut names: Vec<Option<String>> = Vec::with_capacity(idx_chunk.len());
            let mut medias: Vec<Option<String>> = Vec::with_capacity(idx_chunk.len());
            let mut metadatas: Vec<Option<Value>> = Vec::with_capacity(idx_chunk.len());
            let mut minted: Vec<Option<i64>> = Vec::with_capacity(idx_chunk.len());
            let mut owners: Vec<Option<i64>> = Vec::with_capacity(idx_chunk.len());
            let mut owner_ledgers: Vec<Option<i64>> = Vec::with_capacity(idx_chunk.len());

            for &i in idx_chunk {
                let r = &staged.nft_rows[i];
                contracts.push(resolve_contract_id(
                    contract_ids,
                    &r.contract_id,
                    "nft.contract",
                )?);
                token_ids.push(r.token_id.clone());
                collections.push(r.collection_name.clone());
                names.push(r.name.clone());
                medias.push(r.media_url.clone());
                metadatas.push(r.metadata.clone());
                minted.push(r.minted_at_ledger);
                owners.push(resolve_opt_id(
                    account_ids,
                    r.current_owner_str_key.as_deref(),
                    "nft.owner",
                )?);
                owner_ledgers.push(r.current_owner_ledger);
            }

            sqlx::query(
                r#"
                INSERT INTO nfts (
                    contract_id, token_id, collection_name, name, media_url,
                    metadata, minted_at_ledger, current_owner_id, current_owner_ledger
                )
                SELECT * FROM UNNEST(
                    $1::BIGINT[], $2::VARCHAR[], $3::VARCHAR[], $4::VARCHAR[], $5::TEXT[],
                    $6::JSONB[], $7::BIGINT[], $8::BIGINT[], $9::BIGINT[]
                )
                ON CONFLICT (contract_id, token_id) DO UPDATE SET
                  collection_name = COALESCE(EXCLUDED.collection_name, nfts.collection_name),
                  name            = COALESCE(EXCLUDED.name, nfts.name),
                  media_url       = COALESCE(EXCLUDED.media_url, nfts.media_url),
                  metadata        = COALESCE(EXCLUDED.metadata, nfts.metadata),
                  minted_at_ledger = COALESCE(nfts.minted_at_ledger, EXCLUDED.minted_at_ledger),
                  current_owner_id = CASE
                      WHEN EXCLUDED.current_owner_ledger > COALESCE(nfts.current_owner_ledger, 0)
                      THEN EXCLUDED.current_owner_id
                      ELSE nfts.current_owner_id
                  END,
                  current_owner_ledger = GREATEST(
                      COALESCE(nfts.current_owner_ledger, 0), COALESCE(EXCLUDED.current_owner_ledger, 0)
                  )
                "#,
            )
            .bind(&contracts)
            .bind(&token_ids)
            .bind(&collections)
            .bind(&names)
            .bind(&medias)
            .bind(&metadatas)
            .bind(&minted)
            .bind(&owners)
            .bind(&owner_ledgers)
            .execute(&mut **db_tx)
            .await?;
        }
    }

    // 12b. nft_ownership (empty until parser catches up)
    //
    // Iterate surviving indices directly (same allocation win as 12a).
    if !ownership_indices.is_empty() {
        for idx_chunk in ownership_indices.chunks(CHUNK_SIZE) {
            let mut contracts: Vec<i64> = Vec::with_capacity(idx_chunk.len());
            let mut token_ids: Vec<String> = Vec::with_capacity(idx_chunk.len());
            let mut tx_id_vec: Vec<i64> = Vec::with_capacity(idx_chunk.len());
            let mut owners: Vec<Option<i64>> = Vec::with_capacity(idx_chunk.len());
            // ADR 0031: nft_ownership.event_type is SMALLINT (Rust NftEventType).
            let mut types: Vec<NftEventType> = Vec::with_capacity(idx_chunk.len());
            let mut ls_vec: Vec<i64> = Vec::with_capacity(idx_chunk.len());
            let mut order_vec: Vec<i16> = Vec::with_capacity(idx_chunk.len());
            let mut ca_vec: Vec<DateTime<Utc>> = Vec::with_capacity(idx_chunk.len());

            for &i in idx_chunk {
                let r = &staged.nft_ownership_rows[i];
                let Some(tx_id) = tx_ids.get(&r.tx_hash_hex).copied() else {
                    continue;
                };
                contracts.push(resolve_contract_id(
                    contract_ids,
                    &r.contract_id,
                    "nft_ownership.contract",
                )?);
                token_ids.push(r.token_id.clone());
                tx_id_vec.push(tx_id);
                owners.push(resolve_opt_id(
                    account_ids,
                    r.owner_str_key.as_deref(),
                    "nft_ownership.owner",
                )?);
                types.push(r.event_type);
                ls_vec.push(r.ledger_sequence);
                order_vec.push(r.event_order);
                ca_vec.push(r.created_at);
            }
            if contracts.is_empty() {
                continue;
            }

            sqlx::query(
                r#"
                INSERT INTO nft_ownership (
                    nft_id, transaction_id, owner_id, event_type,
                    ledger_sequence, event_order, created_at
                )
                SELECT n.id, tx_id, owner_id, event_type, ls, event_order, ca
                  FROM UNNEST(
                    $1::BIGINT[], $2::VARCHAR[], $3::BIGINT[], $4::BIGINT[],
                    $5::SMALLINT[], $6::BIGINT[], $7::SMALLINT[], $8::TIMESTAMPTZ[]
                  ) AS t(contract_id, token_id, tx_id, owner_id, event_type, ls, event_order, ca)
                  JOIN nfts n ON n.contract_id = t.contract_id AND n.token_id = t.token_id
                ON CONFLICT (nft_id, created_at, ledger_sequence, event_order) DO NOTHING
                "#,
            )
            .bind(&contracts)
            .bind(&token_ids)
            .bind(&tx_id_vec)
            .bind(&owners)
            .bind(&types)
            .bind(&ls_vec)
            .bind(&order_vec)
            .bind(&ca_vec)
            .execute(&mut **db_tx)
            .await?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// 13. liquidity_pools + snapshots + lp_positions
// ---------------------------------------------------------------------------

pub(super) async fn upsert_pools_and_snapshots(
    db_tx: &mut Transaction<'_, Postgres>,
    staged: &Staged,
    account_ids: &HashMap<String, i64>,
) -> Result<(), HandlerError> {
    // 13a. liquidity_pools
    if !staged.pool_rows.is_empty() {
        for chunk in staged.pool_rows.chunks(CHUNK_SIZE) {
            let mut pools: Vec<Vec<u8>> = Vec::with_capacity(chunk.len());
            // ADR 0031: liquidity_pools.asset_*_type are SMALLINT (Rust AssetType).
            let mut a_types: Vec<AssetType> = Vec::with_capacity(chunk.len());
            let mut a_codes: Vec<Option<String>> = Vec::with_capacity(chunk.len());
            let mut a_issuers: Vec<Option<i64>> = Vec::with_capacity(chunk.len());
            let mut b_types: Vec<AssetType> = Vec::with_capacity(chunk.len());
            let mut b_codes: Vec<Option<String>> = Vec::with_capacity(chunk.len());
            let mut b_issuers: Vec<Option<i64>> = Vec::with_capacity(chunk.len());
            let mut fees: Vec<i32> = Vec::with_capacity(chunk.len());
            let mut created_ledgers: Vec<i64> = Vec::with_capacity(chunk.len());

            for r in chunk {
                pools.push(r.pool_id.to_vec());
                a_types.push(r.asset_a_type);
                a_codes.push(r.asset_a_code.clone());
                a_issuers.push(resolve_opt_id(
                    account_ids,
                    r.asset_a_issuer_str_key.as_deref(),
                    "pool.asset_a_issuer",
                )?);
                b_types.push(r.asset_b_type);
                b_codes.push(r.asset_b_code.clone());
                b_issuers.push(resolve_opt_id(
                    account_ids,
                    r.asset_b_issuer_str_key.as_deref(),
                    "pool.asset_b_issuer",
                )?);
                fees.push(r.fee_bps);
                // Pools require created_at_ledger NOT NULL — use last_updated_ledger
                // as the fallback on update-only rows.
                created_ledgers.push(r.created_at_ledger.unwrap_or(r.last_updated_ledger));
            }

            sqlx::query(
                r#"
                INSERT INTO liquidity_pools (
                    pool_id, asset_a_type, asset_a_code, asset_a_issuer_id,
                    asset_b_type, asset_b_code, asset_b_issuer_id,
                    fee_bps, created_at_ledger
                )
                SELECT * FROM UNNEST(
                    $1::BYTEA[], $2::SMALLINT[], $3::VARCHAR[], $4::BIGINT[],
                    $5::SMALLINT[], $6::VARCHAR[], $7::BIGINT[],
                    $8::INTEGER[], $9::BIGINT[]
                )
                ON CONFLICT (pool_id) DO UPDATE SET
                    asset_a_type = liquidity_pools.asset_a_type,
                    created_at_ledger = LEAST(liquidity_pools.created_at_ledger, EXCLUDED.created_at_ledger)
                "#,
            )
            .bind(&pools)
            .bind(&a_types)
            .bind(&a_codes)
            .bind(&a_issuers)
            .bind(&b_types)
            .bind(&b_codes)
            .bind(&b_issuers)
            .bind(&fees)
            .bind(&created_ledgers)
            .execute(&mut **db_tx)
            .await?;
        }
    }

    // 13b. liquidity_pool_snapshots
    if !staged.snapshot_rows.is_empty() {
        for chunk in staged.snapshot_rows.chunks(CHUNK_SIZE) {
            let mut pools: Vec<Vec<u8>> = Vec::with_capacity(chunk.len());
            let mut ls: Vec<i64> = Vec::with_capacity(chunk.len());
            let mut ra: Vec<String> = Vec::with_capacity(chunk.len());
            let mut rb: Vec<String> = Vec::with_capacity(chunk.len());
            let mut ts: Vec<String> = Vec::with_capacity(chunk.len());
            let mut tvl: Vec<Option<String>> = Vec::with_capacity(chunk.len());
            let mut vol: Vec<Option<String>> = Vec::with_capacity(chunk.len());
            let mut fee_rev: Vec<Option<String>> = Vec::with_capacity(chunk.len());
            let mut ca: Vec<DateTime<Utc>> = Vec::with_capacity(chunk.len());

            for r in chunk {
                pools.push(r.pool_id.to_vec());
                ls.push(r.ledger_sequence);
                ra.push(r.reserve_a.clone());
                rb.push(r.reserve_b.clone());
                ts.push(r.total_shares.clone());
                tvl.push(r.tvl.clone());
                vol.push(r.volume.clone());
                fee_rev.push(r.fee_revenue.clone());
                ca.push(r.created_at);
            }

            sqlx::query(
                r#"
                INSERT INTO liquidity_pool_snapshots (
                    pool_id, ledger_sequence, reserve_a, reserve_b, total_shares,
                    tvl, volume, fee_revenue, created_at
                )
                SELECT pool_id, ls, ra::NUMERIC(28,7), rb::NUMERIC(28,7), ts::NUMERIC(28,7),
                       CASE WHEN tvl IS NULL THEN NULL ELSE tvl::NUMERIC(28,7) END,
                       CASE WHEN vol IS NULL THEN NULL ELSE vol::NUMERIC(28,7) END,
                       CASE WHEN fr  IS NULL THEN NULL ELSE fr::NUMERIC(28,7) END,
                       ca
                  FROM UNNEST(
                    $1::BYTEA[], $2::BIGINT[], $3::TEXT[], $4::TEXT[], $5::TEXT[],
                    $6::TEXT[], $7::TEXT[], $8::TEXT[], $9::TIMESTAMPTZ[]
                  ) AS t(pool_id, ls, ra, rb, ts, tvl, vol, fr, ca)
                ON CONFLICT ON CONSTRAINT uq_lp_snapshots_pool_ledger DO NOTHING
                "#,
            )
            .bind(&pools)
            .bind(&ls)
            .bind(&ra)
            .bind(&rb)
            .bind(&ts)
            .bind(&tvl)
            .bind(&vol)
            .bind(&fee_rev)
            .bind(&ca)
            .execute(&mut **db_tx)
            .await?;
        }
    }

    // 13c. lp_positions (empty today)
    if !staged.lp_position_rows.is_empty() {
        for chunk in staged.lp_position_rows.chunks(CHUNK_SIZE) {
            let mut pools: Vec<Vec<u8>> = Vec::with_capacity(chunk.len());
            let mut accts: Vec<i64> = Vec::with_capacity(chunk.len());
            let mut shares: Vec<String> = Vec::with_capacity(chunk.len());
            let mut firsts: Vec<i64> = Vec::with_capacity(chunk.len());
            let mut lasts: Vec<i64> = Vec::with_capacity(chunk.len());

            for r in chunk {
                pools.push(r.pool_id.to_vec());
                accts.push(resolve_id(
                    account_ids,
                    &r.account_str_key,
                    "lp_positions.account",
                )?);
                shares.push(r.shares.clone());
                firsts.push(r.first_deposit_ledger.unwrap_or(r.last_updated_ledger));
                lasts.push(r.last_updated_ledger);
            }

            sqlx::query(
                r#"
                INSERT INTO lp_positions (
                    pool_id, account_id, shares, first_deposit_ledger, last_updated_ledger
                )
                SELECT pool_id, account_id, sh::NUMERIC(28,7), first_d, last_u
                  FROM UNNEST(
                    $1::BYTEA[], $2::BIGINT[], $3::TEXT[], $4::BIGINT[], $5::BIGINT[]
                  ) AS t(pool_id, account_id, sh, first_d, last_u)
                ON CONFLICT (pool_id, account_id) DO UPDATE SET
                    shares = CASE
                        WHEN EXCLUDED.last_updated_ledger >= lp_positions.last_updated_ledger
                        THEN EXCLUDED.shares ELSE lp_positions.shares END,
                    last_updated_ledger = GREATEST(lp_positions.last_updated_ledger, EXCLUDED.last_updated_ledger),
                    first_deposit_ledger = LEAST(lp_positions.first_deposit_ledger, EXCLUDED.first_deposit_ledger)
                "#,
            )
            .bind(&pools)
            .bind(&accts)
            .bind(&shares)
            .bind(&firsts)
            .bind(&lasts)
            .execute(&mut **db_tx)
            .await?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// 14. account_balances_current + account_balance_history + trustline DELETEs
// ---------------------------------------------------------------------------

pub(super) async fn upsert_balances(
    db_tx: &mut Transaction<'_, Postgres>,
    staged: &Staged,
    account_ids: &HashMap<String, i64>,
) -> Result<(), HandlerError> {
    // 14a. DELETE removed trustlines first. Re-creations in the same ledger were
    // already stripped in staging, so anything here is a real removal.
    if !staged.trustline_removals.is_empty() {
        for chunk in staged.trustline_removals.chunks(CHUNK_SIZE) {
            let mut accts: Vec<i64> = Vec::with_capacity(chunk.len());
            let mut codes: Vec<String> = Vec::with_capacity(chunk.len());
            let mut issuers: Vec<i64> = Vec::with_capacity(chunk.len());

            for r in chunk {
                accts.push(resolve_id(
                    account_ids,
                    &r.account_str_key,
                    "balance.remove.account",
                )?);
                codes.push(r.asset_code.clone());
                issuers.push(resolve_id(
                    account_ids,
                    &r.issuer_str_key,
                    "balance.remove.issuer",
                )?);
            }

            sqlx::query(
                r#"
                DELETE FROM account_balances_current abc
                USING UNNEST($1::BIGINT[], $2::VARCHAR[], $3::BIGINT[]) AS t(acct, code, issuer)
                WHERE abc.account_id = t.acct
                  AND abc.asset_code = t.code
                  AND abc.issuer_id  = t.issuer
                  AND abc.asset_type <> 0  -- credit (not native)
                "#,
            )
            .bind(&accts)
            .bind(&codes)
            .bind(&issuers)
            .execute(&mut **db_tx)
            .await?;
        }
    }

    // 14b. account_balances_current upsert — partitioned by identity class to
    // match the partial UNIQUE indexes on (account_id) WHERE native and on
    // (account_id, asset_code, issuer_id) WHERE credit.
    let (natives, credits): (Vec<&BalanceRow>, Vec<&BalanceRow>) = staged
        .balance_rows
        .iter()
        .partition(|r| r.asset_type == AssetType::Native);

    upsert_balances_native(db_tx, &natives, account_ids).await?;
    upsert_balances_credit(db_tx, &credits, account_ids).await?;

    // 14c. account_balance_history — partitioned; append-only, idempotent via
    // partial uidx_abh_native / uidx_abh_credit.
    append_balance_history(db_tx, &staged.balance_history_rows, account_ids).await?;

    Ok(())
}

async fn upsert_balances_native(
    db_tx: &mut Transaction<'_, Postgres>,
    rows: &[&BalanceRow],
    account_ids: &HashMap<String, i64>,
) -> Result<(), HandlerError> {
    if rows.is_empty() {
        return Ok(());
    }
    for chunk in rows.chunks(CHUNK_SIZE) {
        let mut accts: Vec<i64> = Vec::with_capacity(chunk.len());
        let mut bals: Vec<String> = Vec::with_capacity(chunk.len());
        let mut last: Vec<i64> = Vec::with_capacity(chunk.len());

        for r in chunk {
            accts.push(resolve_id(
                account_ids,
                &r.account_str_key,
                "abc.native.account",
            )?);
            bals.push(r.balance.clone());
            last.push(r.last_updated_ledger);
        }

        // ON CONFLICT on the partial UNIQUE index `uidx_abc_native`
        // (account_id WHERE asset_type = 'native'). Watermark rule: only
        // overwrite balance when the incoming ledger is strictly newer.
        sqlx::query(
            r#"
            INSERT INTO account_balances_current
                (account_id, asset_type, asset_code, issuer_id, balance, last_updated_ledger)
            SELECT acct, 0, NULL, NULL, bal::NUMERIC(28,7), last_l   -- AssetType::Native
              FROM UNNEST($1::BIGINT[], $2::TEXT[], $3::BIGINT[]) AS t(acct, bal, last_l)
            ON CONFLICT (account_id) WHERE asset_type = 0   -- native
            DO UPDATE SET
                balance = CASE
                    WHEN EXCLUDED.last_updated_ledger >= account_balances_current.last_updated_ledger
                    THEN EXCLUDED.balance
                    ELSE account_balances_current.balance
                END,
                last_updated_ledger = GREATEST(
                    account_balances_current.last_updated_ledger,
                    EXCLUDED.last_updated_ledger
                )
            "#,
        )
        .bind(&accts)
        .bind(&bals)
        .bind(&last)
        .execute(&mut **db_tx)
        .await?;
    }
    Ok(())
}

async fn upsert_balances_credit(
    db_tx: &mut Transaction<'_, Postgres>,
    rows: &[&BalanceRow],
    account_ids: &HashMap<String, i64>,
) -> Result<(), HandlerError> {
    if rows.is_empty() {
        return Ok(());
    }
    for chunk in rows.chunks(CHUNK_SIZE) {
        let mut accts: Vec<i64> = Vec::with_capacity(chunk.len());
        // ADR 0031: account_balances_current.asset_type is SMALLINT (Rust AssetType).
        let mut types: Vec<AssetType> = Vec::with_capacity(chunk.len());
        let mut codes: Vec<String> = Vec::with_capacity(chunk.len());
        let mut issuers: Vec<i64> = Vec::with_capacity(chunk.len());
        let mut bals: Vec<String> = Vec::with_capacity(chunk.len());
        let mut last: Vec<i64> = Vec::with_capacity(chunk.len());

        for r in chunk {
            let Some(code) = r.asset_code.as_ref() else {
                continue;
            };
            let Some(issuer_key) = r.issuer_str_key.as_ref() else {
                continue;
            };
            accts.push(resolve_id(
                account_ids,
                &r.account_str_key,
                "abc.credit.account",
            )?);
            types.push(r.asset_type);
            codes.push(code.clone());
            issuers.push(resolve_id(account_ids, issuer_key, "abc.credit.issuer")?);
            bals.push(r.balance.clone());
            last.push(r.last_updated_ledger);
        }

        if accts.is_empty() {
            continue;
        }

        // ON CONFLICT on the partial UNIQUE index `uidx_abc_credit`
        // (account_id, asset_code, issuer_id WHERE asset_type <> 'native').
        sqlx::query(
            r#"
            INSERT INTO account_balances_current
                (account_id, asset_type, asset_code, issuer_id, balance, last_updated_ledger)
            SELECT acct, ty, code, issuer, bal::NUMERIC(28,7), last_l
              FROM UNNEST(
                $1::BIGINT[], $2::SMALLINT[], $3::VARCHAR[], $4::BIGINT[], $5::TEXT[], $6::BIGINT[]
              ) AS t(acct, ty, code, issuer, bal, last_l)
            ON CONFLICT (account_id, asset_code, issuer_id) WHERE asset_type <> 0   -- credit (not native)
            DO UPDATE SET
                balance = CASE
                    WHEN EXCLUDED.last_updated_ledger >= account_balances_current.last_updated_ledger
                    THEN EXCLUDED.balance
                    ELSE account_balances_current.balance
                END,
                last_updated_ledger = GREATEST(
                    account_balances_current.last_updated_ledger,
                    EXCLUDED.last_updated_ledger
                ),
                asset_type = account_balances_current.asset_type
            "#,
        )
        .bind(&accts)
        .bind(&types)
        .bind(&codes)
        .bind(&issuers)
        .bind(&bals)
        .bind(&last)
        .execute(&mut **db_tx)
        .await?;
    }
    Ok(())
}

async fn append_balance_history(
    db_tx: &mut Transaction<'_, Postgres>,
    rows: &[BalanceRow],
    account_ids: &HashMap<String, i64>,
) -> Result<(), HandlerError> {
    if rows.is_empty() {
        return Ok(());
    }
    // Partition by identity class — the partial unique indexes
    // (uidx_abh_native / uidx_abh_credit) are disjoint, so each class gets
    // its own INSERT with ON CONFLICT DO NOTHING against the matching index.
    // Replaces the prior anti-join (which scanned account_balance_history
    // per row) with a direct unique-index lookup — O(log n) instead of O(n).
    let (natives, credits): (Vec<&BalanceRow>, Vec<&BalanceRow>) =
        rows.iter().partition(|r| r.asset_type == AssetType::Native);

    append_balance_history_native(db_tx, &natives, account_ids).await?;
    append_balance_history_credit(db_tx, &credits, account_ids).await?;
    Ok(())
}

async fn append_balance_history_native(
    db_tx: &mut Transaction<'_, Postgres>,
    rows: &[&BalanceRow],
    account_ids: &HashMap<String, i64>,
) -> Result<(), HandlerError> {
    if rows.is_empty() {
        return Ok(());
    }
    for chunk in rows.chunks(CHUNK_SIZE) {
        let mut accts: Vec<i64> = Vec::with_capacity(chunk.len());
        let mut ls: Vec<i64> = Vec::with_capacity(chunk.len());
        let mut bals: Vec<String> = Vec::with_capacity(chunk.len());
        let mut ca: Vec<DateTime<Utc>> = Vec::with_capacity(chunk.len());

        for r in chunk {
            accts.push(resolve_id(
                account_ids,
                &r.account_str_key,
                "abh.native.account",
            )?);
            ls.push(r.last_updated_ledger);
            bals.push(r.balance.clone());
            ca.push(r.created_at);
        }

        sqlx::query(
            r#"
            INSERT INTO account_balance_history
                (account_id, ledger_sequence, asset_type, asset_code, issuer_id, balance, created_at)
            SELECT acct, ls, 0, NULL, NULL, bal::NUMERIC(28,7), ca   -- AssetType::Native
              FROM UNNEST($1::BIGINT[], $2::BIGINT[], $3::TEXT[], $4::TIMESTAMPTZ[])
                AS t(acct, ls, bal, ca)
            ON CONFLICT (account_id, ledger_sequence, created_at) WHERE asset_type = 0   -- native
            DO NOTHING
            "#,
        )
        .bind(&accts)
        .bind(&ls)
        .bind(&bals)
        .bind(&ca)
        .execute(&mut **db_tx)
        .await?;
    }
    Ok(())
}

async fn append_balance_history_credit(
    db_tx: &mut Transaction<'_, Postgres>,
    rows: &[&BalanceRow],
    account_ids: &HashMap<String, i64>,
) -> Result<(), HandlerError> {
    if rows.is_empty() {
        return Ok(());
    }
    for chunk in rows.chunks(CHUNK_SIZE) {
        let mut accts: Vec<i64> = Vec::with_capacity(chunk.len());
        let mut ls: Vec<i64> = Vec::with_capacity(chunk.len());
        // ADR 0031: account_balance_history.asset_type is SMALLINT (Rust AssetType).
        let mut types: Vec<AssetType> = Vec::with_capacity(chunk.len());
        let mut codes: Vec<String> = Vec::with_capacity(chunk.len());
        let mut issuers: Vec<i64> = Vec::with_capacity(chunk.len());
        let mut bals: Vec<String> = Vec::with_capacity(chunk.len());
        let mut ca: Vec<DateTime<Utc>> = Vec::with_capacity(chunk.len());

        for r in chunk {
            let Some(code) = r.asset_code.as_ref() else {
                continue;
            };
            let Some(issuer_key) = r.issuer_str_key.as_ref() else {
                continue;
            };
            accts.push(resolve_id(
                account_ids,
                &r.account_str_key,
                "abh.credit.account",
            )?);
            ls.push(r.last_updated_ledger);
            types.push(r.asset_type);
            codes.push(code.clone());
            issuers.push(resolve_id(account_ids, issuer_key, "abh.credit.issuer")?);
            bals.push(r.balance.clone());
            ca.push(r.created_at);
        }
        if accts.is_empty() {
            continue;
        }

        sqlx::query(
            r#"
            INSERT INTO account_balance_history
                (account_id, ledger_sequence, asset_type, asset_code, issuer_id, balance, created_at)
            SELECT acct, ls, ty, code, issuer, bal::NUMERIC(28,7), ca
              FROM UNNEST(
                $1::BIGINT[], $2::BIGINT[], $3::SMALLINT[], $4::VARCHAR[], $5::BIGINT[], $6::TEXT[], $7::TIMESTAMPTZ[]
              ) AS t(acct, ls, ty, code, issuer, bal, ca)
            ON CONFLICT (account_id, ledger_sequence, asset_code, issuer_id, created_at)
              WHERE asset_type <> 0   -- credit (not native)
            DO NOTHING
            "#,
        )
        .bind(&accts)
        .bind(&ls)
        .bind(&types)
        .bind(&codes)
        .bind(&issuers)
        .bind(&bals)
        .bind(&ca)
        .execute(&mut **db_tx)
        .await?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn resolve_id(
    account_ids: &HashMap<String, i64>,
    key: &str,
    field: &'static str,
) -> Result<i64, HandlerError> {
    account_ids
        .get(key)
        .copied()
        .ok_or_else(|| HandlerError::Staging(format!("unresolved StrKey for {field}: {key}")))
}

fn resolve_opt_id(
    account_ids: &HashMap<String, i64>,
    key: Option<&str>,
    field: &'static str,
) -> Result<Option<i64>, HandlerError> {
    match key {
        None => Ok(None),
        Some(k) if !k.starts_with('G') && !k.starts_with('M') => Ok(None),
        Some(k) => Ok(Some(resolve_id(account_ids, k, field)?)),
    }
}

/// Resolve a contract StrKey to its `soroban_contracts.id` surrogate (ADR 0030).
fn resolve_contract_id(
    contract_ids: &HashMap<String, i64>,
    key: &str,
    field: &'static str,
) -> Result<i64, HandlerError> {
    contract_ids.get(key).copied().ok_or_else(|| {
        HandlerError::Staging(format!("unresolved contract StrKey for {field}: {key}"))
    })
}

/// Same as `resolve_contract_id` but tolerant of `None` / non-`C…` inputs.
fn resolve_contract_opt_id(
    contract_ids: &HashMap<String, i64>,
    key: Option<&str>,
    field: &'static str,
) -> Result<Option<i64>, HandlerError> {
    match key {
        None => Ok(None),
        Some(k) if !k.starts_with('C') => Ok(None),
        Some(k) => Ok(Some(resolve_contract_id(contract_ids, k, field)?)),
    }
}
