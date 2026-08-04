//! Shared test support: plan fixtures and the redaction sentinel.

use std::path::Path;

/// The sensitive value embedded in `testdata/plans/sensitive.json`. It
/// must never appear on stdout, stderr, JSON output, or error text.
pub const SENTINEL: &str = "SENTINEL-DO-NOT-PRINT";

pub fn fixture(name: &str) -> serde_json::Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/plans")
        .join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("invalid JSON {}: {e}", path.display()))
}

#[track_caller]
pub fn assert_no_sentinel(context: &str, text: &str) {
    assert!(
        !text.contains(SENTINEL),
        "redaction invariant violated in {context}: sentinel found in output:\n{text}"
    );
}
