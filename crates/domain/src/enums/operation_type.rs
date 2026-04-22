//! Stellar XDR `OperationType` discriminator (Protocol 21 — 27 variants).
//!
//! Maps to `operations.type SMALLINT NOT NULL` with
//! `CHECK (type BETWEEN 0 AND 127)`. Discriminants mirror
//! `stellar_xdr::curr::OperationType` byte-for-byte so parser output can
//! be cast with `as i16` — no lookup, no branch.

use serde::{Deserialize, Serialize};

use super::EnumDecodeError;

/// Stellar operation type. Discriminants match the XDR numbering; the
/// serde representation is the canonical SCREAMING_SNAKE_CASE label used
/// by the Horizon API and historically persisted as VARCHAR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "sqlx", derive(sqlx::Type))]
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[repr(i16)]
pub enum OperationType {
    CreateAccount = 0,
    Payment = 1,
    PathPaymentStrictReceive = 2,
    ManageSellOffer = 3,
    CreatePassiveSellOffer = 4,
    SetOptions = 5,
    ChangeTrust = 6,
    AllowTrust = 7,
    AccountMerge = 8,
    Inflation = 9,
    ManageData = 10,
    BumpSequence = 11,
    ManageBuyOffer = 12,
    PathPaymentStrictSend = 13,
    CreateClaimableBalance = 14,
    ClaimClaimableBalance = 15,
    BeginSponsoringFutureReserves = 16,
    EndSponsoringFutureReserves = 17,
    RevokeSponsorship = 18,
    Clawback = 19,
    ClawbackClaimableBalance = 20,
    SetTrustLineFlags = 21,
    LiquidityPoolDeposit = 22,
    LiquidityPoolWithdraw = 23,
    InvokeHostFunction = 24,
    ExtendFootprintTtl = 25,
    RestoreFootprint = 26,
}

impl OperationType {
    /// Every variant in declaration order. Used by integration tests to
    /// round-trip against `op_type_name(SMALLINT)` SQL and to enumerate
    /// the domain in fixtures.
    pub const VARIANTS: &'static [Self] = &[
        Self::CreateAccount,
        Self::Payment,
        Self::PathPaymentStrictReceive,
        Self::ManageSellOffer,
        Self::CreatePassiveSellOffer,
        Self::SetOptions,
        Self::ChangeTrust,
        Self::AllowTrust,
        Self::AccountMerge,
        Self::Inflation,
        Self::ManageData,
        Self::BumpSequence,
        Self::ManageBuyOffer,
        Self::PathPaymentStrictSend,
        Self::CreateClaimableBalance,
        Self::ClaimClaimableBalance,
        Self::BeginSponsoringFutureReserves,
        Self::EndSponsoringFutureReserves,
        Self::RevokeSponsorship,
        Self::Clawback,
        Self::ClawbackClaimableBalance,
        Self::SetTrustLineFlags,
        Self::LiquidityPoolDeposit,
        Self::LiquidityPoolWithdraw,
        Self::InvokeHostFunction,
        Self::ExtendFootprintTtl,
        Self::RestoreFootprint,
    ];

    /// Canonical label used in API responses and SQL helper output.
    /// Must stay bitwise-equal to `op_type_name(self as i16)` in
    /// `0008_enum_label_functions.sql` — guarded by an integration test.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CreateAccount => "CREATE_ACCOUNT",
            Self::Payment => "PAYMENT",
            Self::PathPaymentStrictReceive => "PATH_PAYMENT_STRICT_RECEIVE",
            Self::ManageSellOffer => "MANAGE_SELL_OFFER",
            Self::CreatePassiveSellOffer => "CREATE_PASSIVE_SELL_OFFER",
            Self::SetOptions => "SET_OPTIONS",
            Self::ChangeTrust => "CHANGE_TRUST",
            Self::AllowTrust => "ALLOW_TRUST",
            Self::AccountMerge => "ACCOUNT_MERGE",
            Self::Inflation => "INFLATION",
            Self::ManageData => "MANAGE_DATA",
            Self::BumpSequence => "BUMP_SEQUENCE",
            Self::ManageBuyOffer => "MANAGE_BUY_OFFER",
            Self::PathPaymentStrictSend => "PATH_PAYMENT_STRICT_SEND",
            Self::CreateClaimableBalance => "CREATE_CLAIMABLE_BALANCE",
            Self::ClaimClaimableBalance => "CLAIM_CLAIMABLE_BALANCE",
            Self::BeginSponsoringFutureReserves => "BEGIN_SPONSORING_FUTURE_RESERVES",
            Self::EndSponsoringFutureReserves => "END_SPONSORING_FUTURE_RESERVES",
            Self::RevokeSponsorship => "REVOKE_SPONSORSHIP",
            Self::Clawback => "CLAWBACK",
            Self::ClawbackClaimableBalance => "CLAWBACK_CLAIMABLE_BALANCE",
            Self::SetTrustLineFlags => "SET_TRUST_LINE_FLAGS",
            Self::LiquidityPoolDeposit => "LIQUIDITY_POOL_DEPOSIT",
            Self::LiquidityPoolWithdraw => "LIQUIDITY_POOL_WITHDRAW",
            Self::InvokeHostFunction => "INVOKE_HOST_FUNCTION",
            Self::ExtendFootprintTtl => "EXTEND_FOOTPRINT_TTL",
            Self::RestoreFootprint => "RESTORE_FOOTPRINT",
        }
    }
}

impl TryFrom<i16> for OperationType {
    type Error = EnumDecodeError;

    fn try_from(v: i16) -> Result<Self, Self::Error> {
        match v {
            0 => Ok(Self::CreateAccount),
            1 => Ok(Self::Payment),
            2 => Ok(Self::PathPaymentStrictReceive),
            3 => Ok(Self::ManageSellOffer),
            4 => Ok(Self::CreatePassiveSellOffer),
            5 => Ok(Self::SetOptions),
            6 => Ok(Self::ChangeTrust),
            7 => Ok(Self::AllowTrust),
            8 => Ok(Self::AccountMerge),
            9 => Ok(Self::Inflation),
            10 => Ok(Self::ManageData),
            11 => Ok(Self::BumpSequence),
            12 => Ok(Self::ManageBuyOffer),
            13 => Ok(Self::PathPaymentStrictSend),
            14 => Ok(Self::CreateClaimableBalance),
            15 => Ok(Self::ClaimClaimableBalance),
            16 => Ok(Self::BeginSponsoringFutureReserves),
            17 => Ok(Self::EndSponsoringFutureReserves),
            18 => Ok(Self::RevokeSponsorship),
            19 => Ok(Self::Clawback),
            20 => Ok(Self::ClawbackClaimableBalance),
            21 => Ok(Self::SetTrustLineFlags),
            22 => Ok(Self::LiquidityPoolDeposit),
            23 => Ok(Self::LiquidityPoolWithdraw),
            24 => Ok(Self::InvokeHostFunction),
            25 => Ok(Self::ExtendFootprintTtl),
            26 => Ok(Self::RestoreFootprint),
            _ => Err(EnumDecodeError::UnknownDiscriminant {
                enum_name: "OperationType",
                value: v,
            }),
        }
    }
}

impl std::str::FromStr for OperationType {
    type Err = EnumDecodeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::VARIANTS
            .iter()
            .copied()
            .find(|v| v.as_str() == s)
            .ok_or_else(|| EnumDecodeError::UnknownLabel {
                enum_name: "OperationType",
                value: s.to_string(),
            })
    }
}

impl std::fmt::Display for OperationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variants_in_declaration_order() {
        assert_eq!(OperationType::VARIANTS.len(), 27);
        for (i, v) in OperationType::VARIANTS.iter().enumerate() {
            assert_eq!(*v as i16, i as i16);
        }
    }

    #[test]
    fn try_from_round_trip() {
        for v in OperationType::VARIANTS {
            assert_eq!(OperationType::try_from(*v as i16).unwrap(), *v);
        }
        assert!(OperationType::try_from(-1).is_err());
        assert!(OperationType::try_from(27).is_err());
    }

    #[test]
    fn from_str_round_trip() {
        for v in OperationType::VARIANTS {
            assert_eq!(v.as_str().parse::<OperationType>().unwrap(), *v);
        }
    }

    #[test]
    fn serde_uses_screaming_snake_case() {
        let json = serde_json::to_string(&OperationType::PathPaymentStrictReceive).unwrap();
        assert_eq!(json, "\"PATH_PAYMENT_STRICT_RECEIVE\"");
        let back: OperationType = serde_json::from_str(&json).unwrap();
        assert_eq!(back, OperationType::PathPaymentStrictReceive);
    }
}
