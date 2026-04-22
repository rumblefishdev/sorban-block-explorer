//! ADR 0031 — single source of truth for every enum-like column.
//!
//! Each column that used to be `VARCHAR(N)` of a closed protocol-defined
//! domain is now `SMALLINT NOT NULL` (or `SMALLINT` nullable for
//! `soroban_contracts.contract_type`) guarded by a `CHECK` range.
//! The Rust enum pins on-disk layout via `#[repr(i16)]`, decodes/encodes
//! as SMALLINT through `sqlx::Type`, and renders the canonical string at
//! the API boundary through serde.
//!
//! Readable SQL labels for psql / BI live in migration `0008_enum_label_functions.sql`.
//! An integration test iterates every variant and asserts that the SQL
//! helper agrees with `Self::as_str()` so drift surfaces in CI, not prod.

mod asset_type;
mod contract_event_type;
mod contract_type;
mod nft_event_type;
mod operation_type;
mod token_asset_type;

pub use asset_type::AssetType;
pub use contract_event_type::ContractEventType;
pub use contract_type::ContractType;
pub use nft_event_type::NftEventType;
pub use operation_type::OperationType;
pub use token_asset_type::TokenAssetType;

/// Error returned when a SMALLINT value read from the database (or a
/// string parsed from an API request) does not correspond to any known
/// variant of the target enum.
///
/// Surfaces as a decode error in sqlx, and as a 400 in API request parsing.
#[derive(Debug, thiserror::Error)]
pub enum EnumDecodeError {
    #[error("unknown {enum_name} discriminant: {value}")]
    UnknownDiscriminant { enum_name: &'static str, value: i16 },
    #[error("unknown {enum_name} label: {value:?}")]
    UnknownLabel {
        enum_name: &'static str,
        value: String,
    },
}
