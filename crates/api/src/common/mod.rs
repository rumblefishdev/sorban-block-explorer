//! Cross-endpoint helpers shared by API modules.
//!
//! Cherry-picked subset for task 0045 — only `errors` is pulled in
//! from the broader `common/*` set delivered by task 0043. The full
//! set (`cursor`, `extractors`, `filters`, `pagination`) lands when
//! task 0043 merges to `develop`; until then `errors` lives here as a
//! pristine copy so the network module can build and ship
//! independently.
//!
//! See task 0043 and ADR 0008.

// `errors` is a pristine cherry-pick from task 0043; only `DB_ERROR`
// + `internal_error` are consumed by the network module today. The
// remaining codes / builders (`INVALID_*`, `bad_request*`, `not_found`)
// are retained as-is so this file stays byte-identical with the 0043
// branch and rebases cleanly once 0043 lands on `develop`. Allow the
// dead-code lint at the module boundary instead of editing the file.
#[allow(dead_code)]
pub mod errors;
