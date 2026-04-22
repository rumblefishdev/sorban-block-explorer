//! Soroban `ContractEventType` discriminator (3 variants).
//!
//! Maps to `soroban_events.event_type SMALLINT NOT NULL` with
//! `CHECK (event_type BETWEEN 0 AND 15)`. Values mirror the XDR enum
//! (`System = 0`, `Contract = 1`, `Diagnostic = 2`).

use serde::{Deserialize, Serialize};

use super::EnumDecodeError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "sqlx", derive(sqlx::Type))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
#[repr(i16)]
pub enum ContractEventType {
    System = 0,
    Contract = 1,
    Diagnostic = 2,
}

impl ContractEventType {
    pub const VARIANTS: &'static [Self] = &[Self::System, Self::Contract, Self::Diagnostic];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Contract => "contract",
            Self::Diagnostic => "diagnostic",
        }
    }
}

impl TryFrom<i16> for ContractEventType {
    type Error = EnumDecodeError;

    fn try_from(v: i16) -> Result<Self, Self::Error> {
        match v {
            0 => Ok(Self::System),
            1 => Ok(Self::Contract),
            2 => Ok(Self::Diagnostic),
            _ => Err(EnumDecodeError::UnknownDiscriminant {
                enum_name: "ContractEventType",
                value: v,
            }),
        }
    }
}

impl std::str::FromStr for ContractEventType {
    type Err = EnumDecodeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::VARIANTS
            .iter()
            .copied()
            .find(|v| v.as_str() == s)
            .ok_or_else(|| EnumDecodeError::UnknownLabel {
                enum_name: "ContractEventType",
                value: s.to_string(),
            })
    }
}

impl std::fmt::Display for ContractEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        for v in ContractEventType::VARIANTS {
            assert_eq!(ContractEventType::try_from(*v as i16).unwrap(), *v);
            assert_eq!(v.as_str().parse::<ContractEventType>().unwrap(), *v);
        }
    }
}
