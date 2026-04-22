//! Explorer-synthetic `tokens.asset_type` domain (4 variants).
//!
//! Maps to `tokens.asset_type SMALLINT NOT NULL`. The variants overlap
//! with XDR `AssetType` on `native` / `classic` but diverge for Soroban
//! assets — an `sac` (Stellar-Asset-Contract-wrapped classic asset) and
//! a pure `soroban` (bespoke contract token) cannot be expressed in the
//! raw XDR discriminator. Kept as a separate enum so each column tells
//! its reader which domain it speaks.

use serde::{Deserialize, Serialize};

use super::EnumDecodeError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "sqlx", derive(sqlx::Type))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
#[repr(i16)]
pub enum TokenAssetType {
    Native = 0,
    Classic = 1,
    Sac = 2,
    Soroban = 3,
}

impl TokenAssetType {
    pub const VARIANTS: &'static [Self] = &[Self::Native, Self::Classic, Self::Sac, Self::Soroban];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Classic => "classic",
            Self::Sac => "sac",
            Self::Soroban => "soroban",
        }
    }
}

impl TryFrom<i16> for TokenAssetType {
    type Error = EnumDecodeError;

    fn try_from(v: i16) -> Result<Self, Self::Error> {
        match v {
            0 => Ok(Self::Native),
            1 => Ok(Self::Classic),
            2 => Ok(Self::Sac),
            3 => Ok(Self::Soroban),
            _ => Err(EnumDecodeError::UnknownDiscriminant {
                enum_name: "TokenAssetType",
                value: v,
            }),
        }
    }
}

impl std::str::FromStr for TokenAssetType {
    type Err = EnumDecodeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::VARIANTS
            .iter()
            .copied()
            .find(|v| v.as_str() == s)
            .ok_or_else(|| EnumDecodeError::UnknownLabel {
                enum_name: "TokenAssetType",
                value: s.to_string(),
            })
    }
}

impl std::fmt::Display for TokenAssetType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        for v in TokenAssetType::VARIANTS {
            assert_eq!(TokenAssetType::try_from(*v as i16).unwrap(), *v);
            assert_eq!(v.as_str().parse::<TokenAssetType>().unwrap(), *v);
        }
    }
}
