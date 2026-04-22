//! Stellar XDR `AssetType` (4 variants).
//!
//! Maps to `liquidity_pools.asset_a_type` / `asset_b_type` and
//! `account_balances_*.asset_type` — every column that carries the raw
//! XDR asset discriminator. `SMALLINT NOT NULL` + `CHECK (… BETWEEN 0 AND 15)`.
//!
//! The serde label is the snake_case form that stellar-xdr emits in its
//! JSON representation — what parser state.rs has always embedded in the
//! `balances` JSON.

use serde::{Deserialize, Serialize};

use super::EnumDecodeError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "sqlx", derive(sqlx::Type))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[repr(i16)]
pub enum AssetType {
    #[serde(rename = "native")]
    Native = 0,
    #[serde(rename = "credit_alphanum4")]
    CreditAlphanum4 = 1,
    #[serde(rename = "credit_alphanum12")]
    CreditAlphanum12 = 2,
    #[serde(rename = "pool_share")]
    PoolShare = 3,
}

impl AssetType {
    pub const VARIANTS: &'static [Self] = &[
        Self::Native,
        Self::CreditAlphanum4,
        Self::CreditAlphanum12,
        Self::PoolShare,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::CreditAlphanum4 => "credit_alphanum4",
            Self::CreditAlphanum12 => "credit_alphanum12",
            Self::PoolShare => "pool_share",
        }
    }
}

impl TryFrom<i16> for AssetType {
    type Error = EnumDecodeError;

    fn try_from(v: i16) -> Result<Self, Self::Error> {
        match v {
            0 => Ok(Self::Native),
            1 => Ok(Self::CreditAlphanum4),
            2 => Ok(Self::CreditAlphanum12),
            3 => Ok(Self::PoolShare),
            _ => Err(EnumDecodeError::UnknownDiscriminant {
                enum_name: "AssetType",
                value: v,
            }),
        }
    }
}

impl std::str::FromStr for AssetType {
    type Err = EnumDecodeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::VARIANTS
            .iter()
            .copied()
            .find(|v| v.as_str() == s)
            .ok_or_else(|| EnumDecodeError::UnknownLabel {
                enum_name: "AssetType",
                value: s.to_string(),
            })
    }
}

impl std::fmt::Display for AssetType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discriminants_match_xdr() {
        assert_eq!(AssetType::Native as i16, 0);
        assert_eq!(AssetType::CreditAlphanum4 as i16, 1);
        assert_eq!(AssetType::CreditAlphanum12 as i16, 2);
        assert_eq!(AssetType::PoolShare as i16, 3);
    }

    #[test]
    fn round_trip() {
        for v in AssetType::VARIANTS {
            assert_eq!(AssetType::try_from(*v as i16).unwrap(), *v);
            assert_eq!(v.as_str().parse::<AssetType>().unwrap(), *v);
        }
    }
}
