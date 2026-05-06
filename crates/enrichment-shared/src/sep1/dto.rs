//! DTOs for the SEP-1 stellar.toml schema slice we consume.
//!
//! Started under task 0188 with a strict scope: only the SEP-1 fields
//! surfaced by `GET /v1/assets/{id}` (`code` / `issuer` / `desc` / and
//! `DOCUMENTATION.ORG_URL`). Task 0191 added `image` to support type-1
//! icon enrichment of `assets.icon_url`. Anything else in the file is
//! silently ignored — keep the parser surface no wider than the
//! consumer surface; add a field here at the same time you add the
//! consumer.
//!
//! TOML keys are case-sensitive: top-level uses upper-case (`CURRENCIES`,
//! `DOCUMENTATION`); inside `[DOCUMENTATION]` keys are SCREAMING_SNAKE_CASE,
//! handled by `serde(rename_all = ...)`.

use serde::Deserialize;

/// Top-level shape of a SEP-1 stellar.toml file.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Sep1TomlParsed {
    /// `[[CURRENCIES]]` — array of per-token records.
    #[serde(default, rename = "CURRENCIES")]
    pub currencies: Vec<Sep1Currency>,
    /// `[DOCUMENTATION]` — issuing organisation info. Only `ORG_URL` is
    /// consumed today (mapped to `home_page` on the asset detail response).
    #[serde(default, rename = "DOCUMENTATION")]
    pub documentation: Option<Sep1Documentation>,
}

/// Per-token entry inside `CURRENCIES`. Only the fields current consumers
/// read are modelled — extend deliberately when a new consumer arrives.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Sep1Currency {
    /// Used to match against the queried asset's code.
    pub code: Option<String>,
    /// Used to match against the queried asset's issuer StrKey.
    pub issuer: Option<String>,
    /// Mapped to `AssetDetailResponse::description` (api type-2, task 0188).
    pub desc: Option<String>,
    /// Mapped to `assets.icon_url` (worker type-1, task 0191).
    pub image: Option<String>,
}

/// `[DOCUMENTATION]` table. Only `ORG_URL` is consumed today (mapped to
/// `AssetDetailResponse::home_page`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub struct Sep1Documentation {
    pub org_url: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_well_formed_stellar_toml() {
        let toml_text = r#"
[[CURRENCIES]]
code = "USDC"
issuer = "GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN"
desc = "Fully reserved fiat-collateralised stablecoin."

[DOCUMENTATION]
ORG_URL = "https://example.com"
"#;
        let parsed: Sep1TomlParsed = toml::from_str(toml_text).expect("parse");
        assert_eq!(parsed.currencies.len(), 1);
        let usdc = &parsed.currencies[0];
        assert_eq!(usdc.code.as_deref(), Some("USDC"));
        assert_eq!(
            usdc.issuer.as_deref(),
            Some("GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN")
        );
        assert_eq!(
            usdc.desc.as_deref(),
            Some("Fully reserved fiat-collateralised stablecoin.")
        );
        let doc = parsed.documentation.expect("documentation block");
        assert_eq!(doc.org_url.as_deref(), Some("https://example.com"));
    }

    #[test]
    fn ignores_unknown_sections_and_keys() {
        // Real stellar.toml files carry plenty of keys we don't model
        // (VALIDATORS, FEDERATION_SERVER, SEP-1 fields outside scope).
        // Parse must succeed and skip everything that doesn't map.
        let toml_text = r#"
NETWORK_PASSPHRASE = "Public Global Stellar Network ; September 2015"
FEDERATION_SERVER = "https://example.com/federation"

[[VALIDATORS]]
ALIAS = "test-1"
PUBLIC_KEY = "GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN"

[[CURRENCIES]]
code = "USDC"
issuer = "GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN"
name = "USD Coin"
image = "https://example.com/usdc.png"
conditions = "Out of scope for the current parser."
display_decimals = 2
"#;
        let parsed: Sep1TomlParsed = toml::from_str(toml_text).expect("parse");
        assert_eq!(parsed.currencies.len(), 1);
        // `name`, `conditions`, `display_decimals` are intentionally
        // not modelled — they are silently dropped. `image` IS modelled
        // (task 0191) and is asserted below alongside the other in-scope
        // fields.
        let usdc = &parsed.currencies[0];
        assert_eq!(usdc.code.as_deref(), Some("USDC"));
        assert_eq!(usdc.image.as_deref(), Some("https://example.com/usdc.png"));
        assert!(usdc.desc.is_none());
        assert!(parsed.documentation.is_none());
    }

    #[test]
    fn empty_file_yields_empty_currencies_and_no_documentation() {
        let parsed: Sep1TomlParsed = toml::from_str("").expect("parse");
        assert!(parsed.currencies.is_empty());
        assert!(parsed.documentation.is_none());
    }

    #[test]
    fn malformed_toml_returns_error() {
        let bad = "[[CURRENCIES\ncode = USDC"; // missing closing bracket + unquoted value
        assert!(toml::from_str::<Sep1TomlParsed>(bad).is_err());
    }
}
