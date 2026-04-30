//! Build-time OpenAPI spec extractor.
//!
//! Prints the current API spec to stdout so callers can redirect it to a file:
//! `cargo run -p api --bin extract_openapi > libs/api-types/src/openapi.json`
//!
//! Reuses [`api::openapi::register_routes`] so the routes advertised here
//! are exactly the routes mounted by the live Lambda app — no chance for
//! the bin and the app to diverge on which endpoints they expose.

fn main() {
    let (_, spec) = api::openapi::register_routes().split_for_parts();

    println!(
        "{}",
        spec.to_pretty_json()
            .expect("failed to serialize OpenAPI spec as pretty JSON")
    );
}
