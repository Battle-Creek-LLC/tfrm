//! J5.1: the R8.1 JSON shapes are contractual — these tests pin the
//! exact key sets documented in docs/cli.md. A schema change fails here,
//! forcing a deliberate doc + test update in the same commit.

mod common;

use common::fixture;
use serde_json::Value;
use tfrm_core::show::RunMeta;

/// Sorted key set (serde_json::Value orders keys alphabetically).
fn keys(value: &Value) -> Vec<&str> {
    let mut keys: Vec<&str> = value
        .as_object()
        .map(|m| m.keys().map(String::as_str).collect())
        .unwrap_or_default();
    keys.sort_unstable();
    keys
}

/// Sorted copy of the documented key list for comparison.
fn sorted<const N: usize>(mut expected: [&str; N]) -> Vec<&str> {
    expected.sort_unstable();
    expected.to_vec()
}

#[test]
fn workspace_row_shape_is_pinned() {
    let row = tfrm_core::workspaces::WorkspaceRow {
        name: "ws".into(),
        current_run_status: Some("applied".into()),
        vcs_repo: Some("acme/ws".into()),
        latest_change_at: Some("2026-08-01T00:00:00Z".into()),
        selected: true,
    };
    let json = serde_json::to_value(&row).unwrap();
    assert_eq!(
        keys(&json),
        sorted([
            "name",
            "current_run_status",
            "vcs_repo",
            "latest_change_at",
            "selected"
        ])
    );
}

#[test]
fn run_row_shape_is_pinned() {
    let row = tfrm_core::runs::RunRow {
        id: "run-1".into(),
        status: "planned".into(),
        created_at: Some("2026-08-01T00:00:00Z".into()),
        commit_sha: Some("abc".into()),
        message: Some("msg".into()),
        source: Some("tfe-api".into()),
        confirmable: true,
    };
    let json = serde_json::to_value(&row).unwrap();
    assert_eq!(
        keys(&json),
        sorted([
            "id",
            "status",
            "created_at",
            "commit_sha",
            "message",
            "source",
            "confirmable"
        ])
    );
}

#[test]
fn show_report_shape_is_pinned() {
    let report = tfrm_core::show::build_report(
        RunMeta {
            id: "run-1".into(),
            workspace: Some("ws".into()),
            status: "planned".into(),
            source: Some("tfe-api".into()),
            commit_sha: Some("abc".into()),
            message: Some("msg".into()),
        },
        &fixture("sensitive.json"),
    );
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(
        keys(&json),
        sorted(["run", "summary", "resource_changes", "output_changes"])
    );
    assert_eq!(
        keys(&json["run"]),
        sorted([
            "id",
            "workspace",
            "status",
            "source",
            "commit_sha",
            "message"
        ])
    );
    assert_eq!(keys(&json["summary"]), sorted(["add", "change", "destroy"]));
    assert_eq!(
        keys(&json["resource_changes"][0]),
        sorted(["address", "action", "attributes"])
    );
    assert_eq!(
        keys(&json["resource_changes"][0]["attributes"][0]),
        sorted(["name", "before", "after"])
    );
    assert_eq!(
        keys(&json["output_changes"][0]),
        sorted(["name", "action", "before", "after"])
    );
}

#[test]
fn diff_report_shape_is_pinned() {
    let report = tfrm_core::diff::diff_plans(
        "run-a",
        "run-b",
        &fixture("sensitive.json"),
        &fixture("sensitive-differ.json"),
        true,
    );
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(
        keys(&json),
        sorted([
            "run_a",
            "run_b",
            "only_in_a",
            "only_in_b",
            "differing",
            "identical_count",
            "identical"
        ])
    );
    assert_eq!(
        keys(&json["differing"][0]),
        sorted(["address", "action_a", "action_b", "attributes"])
    );
    // A sensitive-differs entry has NO value keys — only name + flag.
    assert_eq!(
        keys(&json["differing"][0]["attributes"][0]),
        sorted(["name", "sensitive_differs"])
    );
}
