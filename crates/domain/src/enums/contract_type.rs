//! `soroban_contracts.contract_type` — explorer-synthetic classification
//! of deployed contracts.
//!
//! Maps to `soroban_contracts.contract_type SMALLINT NULL`. Nullable
//! because the two-pass upsert in `persist/write.rs` registers bare
//! StrKey references (from ops / events / invocations / tokens / nfts)
//! before a contract's deployment metadata is observed — those rows
//! start with `contract_type = NULL` and get filled in when the deploy
//! meta lands.
//!
//! CHECK range `BETWEEN 0 AND 15` leaves room for future refinements
//! (e.g. splitting `Other` into `Dex`, `Lending`, `Bridge` …) without a
//! schema migration.

use serde::{Deserialize, Serialize};

use super::EnumDecodeError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "sqlx", derive(sqlx::Type))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
#[repr(i16)]
pub enum ContractType {
    Token = 0,
    Other = 1,
}

impl ContractType {
    pub const VARIANTS: &'static [Self] = &[Self::Token, Self::Other];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Token => "token",
            Self::Other => "other",
        }
    }
}

impl TryFrom<i16> for ContractType {
    type Error = EnumDecodeError;

    fn try_from(v: i16) -> Result<Self, Self::Error> {
        match v {
            0 => Ok(Self::Token),
            1 => Ok(Self::Other),
            _ => Err(EnumDecodeError::UnknownDiscriminant {
                enum_name: "ContractType",
                value: v,
            }),
        }
    }
}

impl std::str::FromStr for ContractType {
    type Err = EnumDecodeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::VARIANTS
            .iter()
            .copied()
            .find(|v| v.as_str() == s)
            .ok_or_else(|| EnumDecodeError::UnknownLabel {
                enum_name: "ContractType",
                value: s.to_string(),
            })
    }
}

impl std::fmt::Display for ContractType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        for v in ContractType::VARIANTS {
            assert_eq!(ContractType::try_from(*v as i16).unwrap(), *v);
            assert_eq!(v.as_str().parse::<ContractType>().unwrap(), *v);
        }
    }
}
