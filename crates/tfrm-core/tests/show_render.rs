//! J2.4: golden-file rendering tests over every plan fixture, plus the
//! redaction sentinel invariant on text and JSON forms.
//!
//! Regenerate goldens with: UPDATE_GOLDEN=1 cargo test -p tfrm-core --test show_render

mod common;

use common::{assert_no_sentinel, fixture};
use serde_json::Value;
use std::path::PathBuf;
use tfrm_core::show::{self, RunMeta};

const FIXTURES: &[&str] = &[
    "create.json",
    "update.json",
    "replace.json",
    "delete.json",
    "sensitive.json",
    "unknown.json",
];

fn meta() -> RunMeta {
    RunMeta {
        id: "run-abc123".into(),
        workspace: Some("platform".into()),
        status: "planned".into(),
        source: Some("tfe-api".into()),
        commit_sha: Some("aaaa1111bbbb2222".into()),
        message: Some("test message".into()),
    }
}

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name)
}

fn check_golden(name: &str, rendered: &str) {
    let path = golden_path(name);
    if std::env::var("UPDATE_GOLDEN").is_ok() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, rendered).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("missing golden {} ({e}); run UPDATE_GOLDEN=1", name));
    pretty_assertions::assert_eq!(expected, rendered, "golden mismatch for {name}");
}

#[test]
fn golden_text_for_every_fixture() {
    for name in FIXTURES {
        let report = show::build_report(meta(), &fixture(name));
        let text = show::render_text(&report);
        check_golden(&name.replace(".json", ".txt"), &text);
        assert_no_sentinel(&format!("{name} text"), &text);
    }
}

/// `--format json` snapshot: re-parsed and walked for the sentinel (R5.3).
#[test]
fn golden_json_for_every_fixture() {
    for name in FIXTURES {
        let report = show::build_report(meta(), &fixture(name));
        let json = serde_json::to_string_pretty(&report).unwrap();
        check_golden(
            &name.replace(".json", ".report.json"),
            &(json.clone() + "\n"),
        );
        assert_no_sentinel(&format!("{name} json"), &json);
        // Walk the re-parsed document too, in case a future serializer
        // escapes the sentinel into a form `contains` would miss.
        let parsed: Value = serde_json::from_str(&json).unwrap();
        walk_no_sentinel(name, &parsed);
    }
}

fn walk_no_sentinel(context: &str, value: &Value) {
    match value {
        Value::String(s) => assert!(
            !s.contains(common::SENTINEL),
            "sentinel in string at {context}: {s}"
        ),
        Value::Array(items) => items.iter().for_each(|v| walk_no_sentinel(context, v)),
        Value::Object(map) => {
            for (k, v) in map {
                assert!(
                    !k.contains(common::SENTINEL),
                    "sentinel in key at {context}"
                );
                walk_no_sentinel(context, v);
            }
        }
        _ => {}
    }
}

#[test]
fn sensitive_values_render_as_markers() {
    let report = show::build_report(meta(), &fixture("sensitive.json"));
    let text = show::render_text(&report);
    assert!(
        text.contains("password: (sensitive) -> (sensitive)"),
        "{text}"
    );
    assert!(
        text.contains("db_password: (sensitive) -> (sensitive)"),
        "{text}"
    );
}

#[test]
fn unknown_values_render_as_known_after_apply() {
    let report = show::build_report(meta(), &fixture("unknown.json"));
    let text = show::render_text(&report);
    assert!(text.contains("public_ip = (known after apply)"), "{text}");
    assert!(text.contains("nat_ip = (known after apply)"), "{text}");
}

#[test]
fn replace_marks_forcing_attribute() {
    let report = show::build_report(meta(), &fixture("replace.json"));
    let text = show::render_text(&report);
    assert!(text.contains("(forced by: engine_version)"), "{text}");
    assert!(text.contains("# forces replacement"), "{text}");
    assert!(
        text.contains("Plan: 1 to add, 0 to change, 1 to destroy."),
        "{text}"
    );
}

#[test]
fn summary_counts_per_fixture() {
    let cases = [
        ("create.json", (1, 0, 0)),
        ("update.json", (0, 1, 0)),
        ("replace.json", (1, 0, 1)),
        ("delete.json", (0, 0, 1)),
    ];
    for (name, (add, change, destroy)) in cases {
        let report = show::build_report(meta(), &fixture(name));
        assert_eq!(
            (
                report.summary.add,
                report.summary.change,
                report.summary.destroy
            ),
            (add, change, destroy),
            "{name}"
        );
    }
}
