//! NFT ownership event kind (3 variants).
//!
//! Parser-internal domain — Soroban does not expose a single canonical
//! XDR enum for NFT transitions; we synthesise `mint` / `transfer` /
//! `burn` from SEP-0041 topic shapes. Maps to
//! `nft_ownership.event_type SMALLINT NOT NULL` with
//! `CHECK (event_type BETWEEN 0 AND 15)`.

use serde::{Deserialize, Serialize};

use super::EnumDecodeError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "sqlx", derive(sqlx::Type))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
#[repr(i16)]
pub enum NftEventType {
    Mint = 0,
    Transfer = 1,
    Burn = 2,
}

impl NftEventType {
    pub const VARIANTS: &'static [Self] = &[Self::Mint, Self::Transfer, Self::Burn];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mint => "mint",
            Self::Transfer => "transfer",
            Self::Burn => "burn",
        }
    }
}

impl TryFrom<i16> for NftEventType {
    type Error = EnumDecodeError;

    fn try_from(v: i16) -> Result<Self, Self::Error> {
        match v {
            0 => Ok(Self::Mint),
            1 => Ok(Self::Transfer),
            2 => Ok(Self::Burn),
            _ => Err(EnumDecodeError::UnknownDiscriminant {
                enum_name: "NftEventType",
                value: v,
            }),
        }
    }
}

impl std::str::FromStr for NftEventType {
    type Err = EnumDecodeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::VARIANTS
            .iter()
            .copied()
            .find(|v| v.as_str() == s)
            .ok_or_else(|| EnumDecodeError::UnknownLabel {
                enum_name: "NftEventType",
                value: s.to_string(),
            })
    }
}

impl std::fmt::Display for NftEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        for v in NftEventType::VARIANTS {
            assert_eq!(NftEventType::try_from(*v as i16).unwrap(), *v);
            assert_eq!(v.as_str().parse::<NftEventType>().unwrap(), *v);
        }
    }
}
