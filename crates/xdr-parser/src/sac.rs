//! SAC contract_id derivation from `ContractIdPreimage`.
//!
//! Per stellar-core:
//!
//! ```text
//! network_id  = SHA256(network_passphrase)
//! contract_id = SHA256(network_id || XDR.serialize(ContractIdPreimage))
//! ```
//!
//! The 32-byte hash is rendered as a `C...` StrKey via `ScAddress`.
//!
//! Canonical passphrases come from the Stellar protocol documentation:
//! <https://developers.stellar.org/docs/data/rpc/api-reference/methods/getNetwork>.

use sha2::{Digest, Sha256};
use stellar_xdr::curr::{
    Asset, ContractId, ContractIdPreimage, CreateContractArgs, CreateContractArgsV2, Hash,
    HashIdPreimage, HashIdPreimageContractId, HostFunction, Limits, OperationBody, ScAddress,
    SorobanAuthorizedFunction, SorobanAuthorizedInvocation, WriteXdr,
};

use crate::envelope::InnerTxRef;
use crate::error::{ParseError, ParseErrorKind};
use crate::types::SacAssetIdentity;

pub const MAINNET_PASSPHRASE: &str = "Public Global Stellar Network ; September 2015";
pub const TESTNET_PASSPHRASE: &str = "Test SDF Network ; September 2015";
pub const FUTURENET_PASSPHRASE: &str = "Test SDF Future Network ; October 2022";

/// Map a logical network name to its canonical passphrase. Case-insensitive.
/// Returns `None` for unknown names so the caller can fail explicitly rather
/// than silently defaulting.
pub fn passphrase_for(network: &str) -> Option<&'static str> {
    match network.to_ascii_lowercase().as_str() {
        "mainnet" | "public" | "pubnet" => Some(MAINNET_PASSPHRASE),
        "testnet" => Some(TESTNET_PASSPHRASE),
        "futurenet" => Some(FUTURENET_PASSPHRASE),
        _ => None,
    }
}

/// `network_id = SHA256(passphrase_bytes)`.
pub fn network_id(passphrase: &str) -> [u8; 32] {
    Sha256::digest(passphrase.as_bytes()).into()
}

/// Derive the SAC `contract_id` StrKey from a `ContractIdPreimage` and the
/// network identifier, matching stellar-core's derivation.
///
/// The hash input is the XDR encoding of the full
/// `HashIdPreimage::ContractId` envelope (tag + network_id + preimage),
/// not the raw preimage alone — stellar-core wraps it that way so the
/// envelope-type discriminator is part of the hash input.
///
/// Returns the 56-char `C...` StrKey.
pub fn derive_sac_contract_id(
    preimage: &ContractIdPreimage,
    network_id: &[u8; 32],
) -> Result<String, ParseError> {
    let envelope = HashIdPreimage::ContractId(HashIdPreimageContractId {
        network_id: Hash(*network_id),
        contract_id_preimage: preimage.clone(),
    });
    let xdr_bytes = envelope.to_xdr(Limits::none()).map_err(|e| ParseError {
        kind: ParseErrorKind::XdrDeserializationFailed,
        message: format!("serialize HashIdPreimage::ContractId: {e}"),
        context: None,
    })?;

    let digest: [u8; 32] = Sha256::digest(&xdr_bytes).into();
    Ok(ScAddress::Contract(ContractId(Hash(digest))).to_string())
}

/// Convert an XDR `Asset` into the corresponding [`SacAssetIdentity`].
fn asset_to_identity(asset: &Asset) -> SacAssetIdentity {
    match asset {
        Asset::Native => SacAssetIdentity::Native,
        Asset::CreditAlphanum4(a) => SacAssetIdentity::Credit {
            code: asset_code_to_string(&a.asset_code.0),
            issuer: a.issuer.0.to_string(),
        },
        Asset::CreditAlphanum12(a) => SacAssetIdentity::Credit {
            code: asset_code_to_string(&a.asset_code.0),
            issuer: a.issuer.0.to_string(),
        },
    }
}

fn asset_code_to_string(bytes: &[u8]) -> String {
    // Asset codes are zero-padded to 4 or 12 bytes; strip trailing NULs so
    // "USDC\0\0\0\0" round-trips to "USDC".
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

fn push_preimage_identity(
    preimage: &ContractIdPreimage,
    network_id: &[u8; 32],
    out: &mut Vec<(String, SacAssetIdentity)>,
) {
    let ContractIdPreimage::Asset(asset) = preimage else {
        return;
    };
    let identity = asset_to_identity(asset);
    match derive_sac_contract_id(preimage, network_id) {
        Ok(contract_id) => out.push((contract_id, identity)),
        Err(e) => tracing::warn!(error = %e.message, "derive_sac_contract_id failed"),
    }
}

fn walk_auth_node(
    node: &SorobanAuthorizedInvocation,
    network_id: &[u8; 32],
    out: &mut Vec<(String, SacAssetIdentity)>,
) {
    match &node.function {
        SorobanAuthorizedFunction::CreateContractHostFn(CreateContractArgs {
            contract_id_preimage,
            ..
        }) => push_preimage_identity(contract_id_preimage, network_id, out),
        SorobanAuthorizedFunction::CreateContractV2HostFn(CreateContractArgsV2 {
            contract_id_preimage,
            ..
        }) => push_preimage_identity(contract_id_preimage, network_id, out),
        SorobanAuthorizedFunction::ContractFn(_) => {}
    }
    for child in node.sub_invocations.iter() {
        walk_auth_node(child, network_id, out);
    }
}

/// Collect all SAC `(contract_id, identity)` pairs reachable from a single
/// transaction envelope — both top-level `CreateContract` host-function
/// operations AND `CreateContractHostFn` auth entries (factory pattern).
///
/// Each `contract_id` is derived from the preimage via
/// [`derive_sac_contract_id`] (stellar-core convention), so downstream
/// persistence can key off a deterministic, batch-independent identifier
/// rather than `tx_hash` correlation.
pub fn extract_sac_identities(
    envelope: &InnerTxRef<'_>,
    network_id: &[u8; 32],
) -> Vec<(String, SacAssetIdentity)> {
    let ops = match envelope {
        InnerTxRef::V0(tx) => tx.operations.as_slice(),
        InnerTxRef::V1(tx) => tx.operations.as_slice(),
    };
    let mut out = Vec::new();
    for op in ops {
        let OperationBody::InvokeHostFunction(ref invoke) = op.body else {
            continue;
        };
        match &invoke.host_function {
            HostFunction::CreateContract(args) => {
                push_preimage_identity(&args.contract_id_preimage, network_id, &mut out);
            }
            HostFunction::CreateContractV2(args) => {
                push_preimage_identity(&args.contract_id_preimage, network_id, &mut out);
            }
            _ => {}
        }
        for auth_entry in invoke.auth.iter() {
            walk_auth_node(&auth_entry.root_invocation, network_id, &mut out);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_xdr::curr::{AccountId, AlphaNum4, Asset, AssetCode4};

    /// Mainnet network_id is a well-known hex string; drift in SHA2 or
    /// passphrase definition would be caught here before anything else
    /// misbehaves.
    #[test]
    fn mainnet_network_id_matches_known_hex() {
        assert_eq!(
            hex::encode(network_id(MAINNET_PASSPHRASE)),
            "7ac33997544e3175d266bd022439b22cdb16508c01163f26e5cb2a3e1045a979"
        );
    }

    #[test]
    fn testnet_network_id_matches_known_hex() {
        assert_eq!(
            hex::encode(network_id(TESTNET_PASSPHRASE)),
            "cee0302d59844d32bdca915c8203dd44b33fbb7edc19051ea37abedf28ecd472"
        );
    }

    #[test]
    fn passphrase_lookup_accepts_common_aliases() {
        assert_eq!(passphrase_for("mainnet"), Some(MAINNET_PASSPHRASE));
        assert_eq!(passphrase_for("MAINNET"), Some(MAINNET_PASSPHRASE));
        assert_eq!(passphrase_for("public"), Some(MAINNET_PASSPHRASE));
        assert_eq!(passphrase_for("testnet"), Some(TESTNET_PASSPHRASE));
        assert_eq!(passphrase_for("bogus"), None);
    }

    /// XLM-SAC on mainnet is a published constant across the Stellar
    /// ecosystem (Horizon, SDK, Stellar Expert). Regression-guards the
    /// derivation against any change in XDR layout or hashing inputs.
    #[test]
    fn xlm_sac_mainnet_contract_id() {
        let net = network_id(MAINNET_PASSPHRASE);
        let preimage = ContractIdPreimage::Asset(Asset::Native);
        let cid = derive_sac_contract_id(&preimage, &net).unwrap();
        assert_eq!(
            cid,
            "CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA"
        );
    }

    /// Circle USDC mainnet SAC: issuer `GA5ZSEJY...KZVN`, code `USDC`.
    #[test]
    fn usdc_sac_mainnet_contract_id() {
        use core::str::FromStr;
        let issuer =
            AccountId::from_str("GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN")
                .unwrap();
        let asset = Asset::CreditAlphanum4(AlphaNum4 {
            asset_code: AssetCode4(*b"USDC"),
            issuer,
        });

        let net = network_id(MAINNET_PASSPHRASE);
        let preimage = ContractIdPreimage::Asset(asset);
        let cid = derive_sac_contract_id(&preimage, &net).unwrap();
        assert_eq!(
            cid,
            "CCW67TSZV3SSS2HXMBQ5JFGCKJNXKZM7UQUWUZPUTHXSTZLEO7SJMI75"
        );
    }
}
