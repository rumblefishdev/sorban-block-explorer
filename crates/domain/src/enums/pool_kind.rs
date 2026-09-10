//! `liquidity_pools.pool_kind` domain (2 variants): which PROTOCOL a pool
//! belongs to, and therefore which of the table's two column sets applies to
//! it and how its 32-byte id renders.
//!
//! The column existed before this enum did, as a bare `UInt8` with no named
//! vocabulary — the shape ADR 0031 exists to retire. It is enum-like by
//! construction (a closed set, fixed by the schema) and it is load-bearing on
//! the wire: the same 32 bytes render as a SEP-23 `L…` strkey for a classic
//! pool and a `C…` contract address for a soroban one, and rendering one as
//! the other produces a **well-formed WRONG key** rather than an error.
//!
//! Deliberately NOT collapsed into `AssetFamily` or any asset enum: a pool's
//! kind says which protocol created the pool, not what its legs are. A soroban
//! pool can hold classic assets (measured: most of them do, through their
//! SACs), so the two vocabularies answer different questions and must not
//! share a value.

use serde::{Deserialize, Serialize};

use super::EnumDecodeError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
#[repr(i16)]
pub enum PoolKind {
    /// A protocol liquidity pool (CAP-38). Its id is a SEP-23 `L…` strkey and
    /// its legs are classic assets.
    Classic = 0,
    /// A pool deployed as a Soroban contract. Its id is the contract address,
    /// and its legs are token contracts — which for most pools measured are
    /// the SACs of classic assets rather than bespoke tokens.
    Soroban = 1,
}

impl PoolKind {
    pub const VARIANTS: &'static [Self] = &[Self::Classic, Self::Soroban];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Classic => "classic",
            Self::Soroban => "soroban",
        }
    }
}

impl TryFrom<i16> for PoolKind {
    type Error = EnumDecodeError;

    fn try_from(v: i16) -> Result<Self, Self::Error> {
        match v {
            0 => Ok(Self::Classic),
            1 => Ok(Self::Soroban),
            _ => Err(EnumDecodeError::UnknownDiscriminant {
                enum_name: "PoolKind",
                value: v,
            }),
        }
    }
}

impl std::str::FromStr for PoolKind {
    type Err = EnumDecodeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::VARIANTS
            .iter()
            .copied()
            .find(|v| v.as_str() == s)
            .ok_or_else(|| EnumDecodeError::UnknownLabel {
                enum_name: "PoolKind",
                value: s.to_string(),
            })
    }
}

impl std::fmt::Display for PoolKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        for v in PoolKind::VARIANTS {
            assert_eq!(PoolKind::try_from(*v as i16).unwrap(), *v);
            assert_eq!(v.as_str().parse::<PoolKind>().unwrap(), *v);
        }
    }

    /// The two labels must not collide with the asset vocabulary: `soroban`
    /// appears in both, and a renderer that mixes them is the 0496 incident
    /// again. Pinned so a rename here has to think about it.
    #[test]
    fn soroban_means_the_pool_here_not_the_asset() {
        assert_eq!(PoolKind::Soroban.as_str(), "soroban");
        assert_eq!(domain_asset_family_soroban(), "soroban");
    }

    fn domain_asset_family_soroban() -> &'static str {
        super::super::AssetFamily::Soroban.as_str()
    }
}
