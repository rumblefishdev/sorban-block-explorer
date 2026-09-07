//! Wire shapes mirror canonical SQL `endpoint-queries-clickhouse/{06,07}_*.sql`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// Query params for `GET /v1/accounts` (list). `filter[with_domain]` keeps
/// only accounts that set a `home_domain` (known/anchor accounts).
///
/// No `filter[q]`: account StrKeys are opaque base32 (not human-meaningful),
/// so an address prefix is a useless browse filter; exact-address lookup is
/// the global search's job (`/v1/search` redirects a full G-strkey straight
/// to the account detail page).
#[derive(Debug, Deserialize, IntoParams)]
pub struct AccountsListParams {
    /// Base sort order on `last_seen_ledger`: `asc` | `desc` (default).
    /// Sticky — re-send on every page alongside `cursor`.
    pub order: Option<String>,
    #[serde(rename = "filter[with_domain]")]
    pub filter_with_domain: Option<bool>,
}

/// Query params for `GET /v1/accounts/{account_id}/transactions`.
/// `order` controls the sort direction on `created_at` (PG) /
/// `ledger_sequence` (CH). Default is `desc` (newest-first).
#[derive(Debug, Deserialize, IntoParams)]
pub struct AccountTxListParams {
    /// Sort order on transaction time: `asc` | `desc` (default).
    /// Sticky — re-send on every page alongside `cursor`.
    pub order: Option<String>,
}

/// One row of `GET /v1/accounts`. Identity + native (XLM) balance + the
/// first/last-seen activity window + `home_domain`. Ordered by
/// `last_seen_ledger` (the only indexed sort). `xlm_balance` is the native
/// balance from the unified `balances` table; `null` if no native row exists.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AccountListItem {
    pub account_id: String,
    /// Native (XLM) balance as a RAW `Int128` stroop string (human value =
    /// ÷10⁷); the frontend scales by 7 decimals — same raw-amount contract as
    /// the account-detail balances.
    pub xlm_balance: Option<String>,
    pub last_seen_ledger: i64,
    pub first_seen_ledger: i64,
    pub home_domain: Option<String>,
}

/// Native rows have `null` `asset_code` / `asset_issuer`; classic credit rows have
/// both; Soroban (type-3) token rows carry `contract_id` + on-chain `name`/`symbol`
/// instead (the account portfolio includes Soroban balances, task 0331 Option C).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AccountBalance {
    /// Label in the `AssetFamily` vocabulary: `native` | `classic_credit` |
    /// `soroban`. (Task 0496: this used to borrow the XDR legend, which calls
    /// 3 `pool_share` — so every Soroban holding read as an LP share. This
    /// field's values come from `assets.asset_type`, whose domain is the
    /// family enum, and the label now follows that domain.)
    pub asset_type_name: Option<String>,
    /// Raw SMALLINT — stable across label renames.
    #[serde(rename = "type")]
    pub asset_type: i16,
    pub asset_code: Option<String>,
    pub asset_issuer: Option<String>,
    /// C-strkey of the Soroban token's contract (type-3 only) — the identity +
    /// link target (`/assets/${contract_id}`). `null` for native / classic.
    pub contract_id: Option<String>,
    /// Asset display `name`, from two disjoint sources by asset type: classic /
    /// native → off-chain SEP-1 enrichment (`asset_enrichment`, only ~3% of classic
    /// assets carry one); Soroban (type-3) → on-chain `METADATA` (e.g. "USDC-EURC
    /// Soroswap LP Token", 100% coverage). Distinct from the `symbol` ticker.
    /// `null` when neither source has a name.
    pub name: Option<String>,
    /// On-chain token `symbol` (type-3, from `METADATA`, e.g. "SMOL") — the short
    /// ticker. `null` for native / classic (they carry `asset_code`).
    pub symbol: Option<String>,
    /// RAW integer balance as a string (`Int128`) — scale by `decimals` (task 0331
    /// Option C: one convention for all asset types; classic `decimals` = 7). The
    /// account portfolio now includes Soroban (type-3) token balances too.
    pub balance: String,
    /// Display decimals — 7 for classic, on-chain `METADATA` for Soroban tokens.
    pub decimals: u32,
    pub last_updated_ledger: i64,
    /// Whether this asset has a Stellar Asset Contract DEPLOYED on-chain
    /// (ADR 0051). A SAC is a PROPERTY of a classic/native asset, orthogonal to
    /// its type — the same axis `/assets` renders as a separate tag beside the
    /// type badge, and the same `asset_sac` join feeds both.
    ///
    /// It is a fact about the ASSET, never about the issuer: the account page
    /// previously inferred it from the issuer address starting with `C`, which
    /// cannot happen (`asset_issuer` resolves out of `accounts`, and no account
    /// has a contract address), so that badge never rendered once.
    ///
    /// `false` also covers a RESERVED-but-undeployed SAC — an address exists,
    /// a contract does not, so nothing links.
    pub sac_deployed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AccountDetailResponse {
    pub account_id: String,
    pub sequence_number: i64,
    pub balances: Vec<AccountBalance>,
    pub home_domain: Option<String>,
    pub first_seen_ledger: i64,
    pub last_seen_ledger: i64,
    /// `true` when the account was removed from the ledger via `account_merge`
    /// and never re-funded (its last lifecycle event is the merge). Derived,
    /// not stored.
    pub deleted: bool,
    /// Signing configuration, or `null` when we have never observed this
    /// account's entry state.
    ///
    /// **`null` is NOT "no extra signers".** 25% of indexed accounts carry no
    /// row, so a client that renders `null` as "single signature" is making a
    /// security claim the data does not support. Render the unknown as
    /// unknown.
    pub signing: Option<AccountSigning>,
}

/// One entry of an account's signer list, exactly as the ledger stores it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AccountSigner {
    /// The signer's StrKey. `ed25519` signers are `G…`; the hash types carry
    /// their own prefixes.
    pub key: String,
    /// 1–255 for a real signer (`SetOptions` deletes at 0). Stored as the
    /// chain carried it — an out-of-range value is an anomaly, never clamped.
    pub weight: u32,
    /// `ed25519` | `preauth_tx` | `hash_x` (the only three the network has).
    #[serde(rename = "type")]
    pub signer_type: String,
}

/// An account's signing configuration: who may sign, and how much weight each
/// operation class demands.
///
/// **The master key is NOT in `signers`.** The ledger keeps the account's own
/// key out of that list and expresses its weight as `master_weight`; Horizon
/// synthesises an extra entry, we do not. A client showing only `signers`
/// therefore reads 3-of-4 where the chain says 3-of-5 — compose the master key
/// into the display, do not expect it here.
///
/// A `master_weight` of 0 is a permanently DISABLED master key, not a signer
/// with a low weight. It is a common, deliberate configuration.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AccountSigning {
    pub signers: Vec<AccountSigner>,
    /// Weight of the account's own key — compare thresholds against
    /// `master_weight` PLUS the sum of `signers[].weight`.
    pub master_weight: u32,
    pub threshold_low: u32,
    pub threshold_med: u32,
    pub threshold_high: u32,
    /// Ledger at which this configuration was last observed.
    pub last_updated_ledger: i64,
}

/// Slim — `inner_tx_hash` lives on `/v1/transactions` only.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AccountTransactionItem {
    /// 64-char lowercase hex.
    pub hash: String,
    pub ledger_sequence: i64,
    /// 1-based position in ledger.
    pub application_order: i16,
    pub source_account: String,
    /// Fee charged, in raw stroops. Native (XLM) is always 7 decimals, so
    /// there is no `decimals` field — the frontend scales by 1e7.
    pub fee_charged: i64,
    pub successful: bool,
    pub operation_count: i16,
    pub has_soroban: bool,
    pub operation_types: Vec<String>,
    pub created_at: DateTime<Utc>,
}
/// Keyset cursor for the accounts list — sorts on `last_seen_ledger` with the
/// surrogate `id` as the unique tiebreak. Serialized (base64/JSON) into the
/// opaque wire cursor (ADR 0008), so it lives on the DTO boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountsListCursor {
    pub last_seen_ledger: i64,
    pub id: i64,
}
