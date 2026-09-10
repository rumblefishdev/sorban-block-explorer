//! Matching liquidity pools by asset code — the ONE definition, shared by the
//! pools list (`/v1/liquidity-pools`) and global search (`/v1/search`).
//!
//! It lives here because the two endpoints answered the same question
//! differently: task 0440 taught the pools list substring + `A/B` pair
//! matching, and global search kept matching pools on an exact `pool_id`
//! only, so `KALE` returned 58 pools on one surface and 0 on the other
//! (task 0470). A second copy of this rule would drift the same way — and
//! the native case below is precisely where a re-implementation goes wrong.

/// Split a free-text asset filter into at most two needles.
///
/// Stellar protocol asset codes are case-sensitive (1–12 ASCII chars, any
/// case), but the canonical convention is uppercase (USDC, XLM). The
/// trim+uppercase normalization matches caller intent for a free-text field;
/// consumers who need exact case-sensitive issuer-disambiguated matching
/// should use a per-leg `(code, issuer)` mode instead.
///
/// `splitn(2, '/')` caps the result at two: a third slash stays inside the
/// second needle rather than silently becoming an extra constraint.
pub fn normalize_asset_codes(raw: Option<String>) -> Vec<String> {
    raw.map(|s| s.trim().to_uppercase())
        .into_iter()
        .flat_map(|s| {
            s.splitn(2, '/')
                .map(|part| part.trim().to_string())
                .collect::<Vec<_>>()
        })
        .filter(|s| !s.is_empty())
        .collect()
}

/// The set of `assets.id` whose DISPLAYED code contains the needle.
///
/// The display rule itself comes from `common::asset_identity` — the assets
/// list and global search rank on the same expression, and this used to be a
/// third hand-written copy of it.
///
/// The set is matched against `legs`, which both pool kinds fill with asset
/// surrogates, so one rule answers for both. The pair columns it used to read
/// are classic-only, which is why the same filter used to return every soroban
/// pool as a false `XLM` hit: their placeholder leg types read as native.
fn matching_assets() -> String {
    format!(
        "SELECT id FROM assets WHERE position({shown}, lower(?)) > 0",
        shown = crate::common::asset_identity::shown_code_sql(""),
    )
}

/// Boolean expression matching pools against `codes`, plus its bind values in
/// left-to-right `?` order. `None` when there is nothing to match on — the
/// caller then adds no clause at all.
///
/// A pair assigns each needle its OWN leg, rather than asking each needle
/// independently whether it matches somewhere. The difference only shows when
/// the needles overlap, and then it is the whole answer: `USDC/USDC` means the
/// pools with USDC on both sides, not every pool with USDC anywhere. Same for a
/// needle that is a prefix of the other (`USD/USDC`) — one leg must not satisfy
/// both halves.
///
/// Over a pair that was two columns; over a list it is Hall's condition for two
/// sets, which for legs reads: something matches the first needle, something
/// matches the second, and at least TWO legs match either. Without the last
/// clause a single USDC leg would satisfy `USDC/USDC` on its own.
pub fn asset_codes_predicate(codes: &[String]) -> Option<(String, Vec<String>)> {
    let set = matching_assets();
    // The two halves of a pair carry the SAME expression — only the bind
    // distinguishes them, which is why this is one string and not an `a` and
    // a `b` that a reader has to diff before finding they are identical.
    let has_leg = format!("arrayExists(x -> x IN ({set}), lp.legs)");
    match codes {
        [one] => Some((has_leg, vec![one.clone()])),
        [first, second] => Some((
            format!(
                "({has_leg} AND {has_leg} \
                  AND arrayCount(x -> x IN ({set}) OR x IN ({set}), lp.legs) >= 2)"
            ),
            // Bind order follows the `?`s left to right: the two `arrayExists`
            // sets, then the two inside the count.
            vec![first.clone(), second.clone(), first.clone(), second.clone()],
        )),
        // `normalize_asset_codes` yields at most two needles; zero means no
        // filter was asked for.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(raw: &str) -> Vec<String> {
        normalize_asset_codes(Some(raw.to_string()))
    }

    #[test]
    fn uppercases_and_trims() {
        assert_eq!(codes("  usdc "), vec!["USDC"]);
    }

    #[test]
    fn splits_a_pair_on_the_slash() {
        assert_eq!(codes("xlm/kale"), vec!["XLM", "KALE"]);
    }

    #[test]
    fn third_code_stays_inside_the_second_needle() {
        // Not three constraints — the second needle keeps the rest verbatim,
        // so the query matches nothing rather than quietly dropping a code.
        assert_eq!(codes("a/b/c"), vec!["A", "B/C"]);
    }

    #[test]
    fn empty_and_blank_yield_no_needles() {
        assert!(codes("").is_empty());
        assert!(codes("   ").is_empty());
        assert!(codes("/").is_empty());
        assert!(normalize_asset_codes(None).is_empty());
    }

    #[test]
    fn no_needles_means_no_clause() {
        assert!(asset_codes_predicate(&[]).is_none());
    }

    #[test]
    fn a_single_needle_asks_the_legs_once() {
        let (sql, binds) = asset_codes_predicate(&codes("kale")).expect("clause");
        assert_eq!(binds, vec!["KALE"]);
        assert_eq!(sql.matches('?').count(), 1);
        assert!(sql.contains("arrayExists"), "{sql}");
        assert!(
            !sql.contains("arrayCount"),
            "one needle needs no distinctness"
        );
    }

    /// Order does not matter because neither needle is tied to a position —
    /// they are tied to DISTINCT legs, which the count enforces.
    #[test]
    fn a_pair_needs_two_distinct_legs() {
        let (sql, binds) = asset_codes_predicate(&codes("xlm/kale")).expect("clause");
        assert_eq!(binds, vec!["XLM", "KALE", "XLM", "KALE"]);
        assert_eq!(sql.matches('?').count(), 4);
        assert!(
            sql.contains(">= 2"),
            "the distinctness clause is missing: {sql}"
        );
    }

    /// The case the distinctness clause exists for: one leg cannot answer both
    /// halves of `USDC/USDC`.
    #[test]
    fn a_repeated_needle_still_needs_two_legs() {
        let (sql, binds) = asset_codes_predicate(&codes("usdc/usdc")).expect("clause");
        assert_eq!(binds, vec!["USDC", "USDC", "USDC", "USDC"]);
        assert!(sql.contains("arrayCount"), "{sql}");
    }

    /// Load-bearing: without the `type = 0` arm, `XLM` matches impostor codes
    /// and misses every real XLM pool (task 0440).
    #[test]
    fn native_is_matched_by_type_not_by_code() {
        let (sql, _) = asset_codes_predicate(&codes("xlm")).expect("clause");
        assert!(
            sql.contains("if(asset_type = 0, 'XLM'"),
            "the native alias is gone: {sql}"
        );
    }

    /// The predicate reads `legs`, never the legacy pair columns — which is
    /// what makes it answer for a soroban pool at all.
    #[test]
    fn it_reads_legs_and_not_the_pair_columns() {
        let (sql, _) = asset_codes_predicate(&codes("usdc")).expect("clause");
        assert!(sql.contains("lp.legs"), "{sql}");
        assert!(!sql.contains("asset_a_"), "{sql}");
        assert!(!sql.contains("asset_b_"), "{sql}");
    }
}
