//! J2.1: `workspace list` (2-page fixture, columns, selected marker) and
//! `workspace select` (404 → exit 4, success persists selection).

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Project dir pointing tfrm at the mock server; auth via --token.
fn project(server_uri: &str, extra: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(".tfrm.toml"),
        format!("org = \"acme\"\nhostname = \"{server_uri}\"\n{extra}"),
    )
    .unwrap();
    dir
}

fn tfrm_in(dir: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("tfrm").unwrap();
    cmd.current_dir(dir).args(["--token", "test-token"]);
    cmd
}

async fn run_blocking(mut cmd: Command) -> assert_cmd::assert::Assert {
    tokio::task::spawn_blocking(move || cmd.assert())
        .await
        .unwrap()
}

fn workspace(id: &str, name: &str, run: Option<&str>, repo: Option<&str>) -> serde_json::Value {
    json!({
        "id": id,
        "type": "workspaces",
        "attributes": {
            "name": name,
            "latest-change-at": format!("2026-08-0{}T12:00:00Z", id.len() % 9 + 1),
            "vcs-repo": repo.map(|r| json!({"identifier": r})),
        },
        "relationships": {
            "current-run": {"data": run.map(|r| json!({"id": r, "type": "runs"}))}
        }
    })
}

async fn mount_two_pages(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/api/v2/organizations/acme/workspaces"))
        .and(query_param("include", "current_run"))
        .and(query_param("page[number]", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [
                workspace("ws-one", "networking", Some("run-1"), Some("acme/networking")),
                workspace("ws-two", "platform", Some("run-2"), None),
            ],
            "included": [
                {"id": "run-1", "type": "runs", "attributes": {"status": "applied"}},
                {"id": "run-2", "type": "runs", "attributes": {"status": "planned"}},
            ],
            "meta": {"pagination": {"current-page": 1, "next-page": 2, "total-pages": 2}}
        })))
        .expect(1)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/organizations/acme/workspaces"))
        .and(query_param("page[number]", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [
                workspace("ws-three", "billing", None, Some("acme/billing")),
            ],
            "included": [],
            "meta": {"pagination": {"current-page": 2, "next-page": null, "total-pages": 2}}
        })))
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn list_renders_columns_and_selected_marker() {
    let server = MockServer::start().await;
    mount_two_pages(&server).await;
    // `platform` is selected via persisted selection.
    let dir = project(&server.uri(), "");
    std::fs::create_dir_all(dir.path().join(".tfrm")).unwrap();
    std::fs::write(
        dir.path().join(".tfrm/local.toml"),
        "workspace = \"platform\"\n",
    )
    .unwrap();

    let mut cmd = tfrm_in(dir.path());
    cmd.args(["workspace", "list"]);
    let assert = run_blocking(cmd).await;
    let out = String::from_utf8(assert.success().get_output().stdout.clone()).unwrap();

    for header in ["NAME", "RUN STATUS", "VCS REPO", "LATEST CHANGE"] {
        assert!(out.contains(header), "missing header {header}:\n{out}");
    }
    for cell in [
        "networking",
        "platform",
        "billing",
        "applied",
        "planned",
        "acme/networking",
    ] {
        assert!(out.contains(cell), "missing cell {cell}:\n{out}");
    }
    let marked: Vec<&str> = out.lines().filter(|l| l.starts_with('*')).collect();
    assert_eq!(marked.len(), 1, "exactly one selected row:\n{out}");
    assert!(marked[0].contains("platform"), "{out}");
}

#[tokio::test(flavor = "multi_thread")]
async fn select_unknown_workspace_exits_4() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/organizations/acme/workspaces/ghost"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "errors": [{"status": "404", "title": "not found"}]
        })))
        .mount(&server)
        .await;

    let dir = project(&server.uri(), "");
    let mut cmd = tfrm_in(dir.path());
    cmd.args(["workspace", "select", "ghost"]);
    let assert = run_blocking(cmd).await;
    assert
        .code(4)
        .stderr(predicate::str::contains("ghost").and(predicate::str::contains("acme")));
    assert!(!dir.path().join(".tfrm/local.toml").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn select_persists_then_current_reports_selection() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/organizations/acme/workspaces/platform"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": workspace("ws-two", "platform", None, None)
        })))
        .mount(&server)
        .await;

    let dir = project(&server.uri(), "");
    let mut cmd = tfrm_in(dir.path());
    cmd.args(["workspace", "select", "platform"]);
    let assert = run_blocking(cmd).await;
    assert
        .success()
        .stdout(predicate::str::contains("selected workspace platform"));

    let mut cmd = tfrm_in(dir.path());
    cmd.args(["workspace", "current"]);
    let assert = run_blocking(cmd).await;
    assert
        .success()
        .stdout(predicate::str::contains("platform").and(predicate::str::contains("selection")));
}

#[tokio::test(flavor = "multi_thread")]
async fn list_format_json_emits_the_documented_shape() {
    let server = MockServer::start().await;
    mount_two_pages(&server).await;
    let dir = project(&server.uri(), "workspace = \"billing\"\n");

    let mut cmd = tfrm_in(dir.path());
    cmd.args(["workspace", "list", "--format", "json"]);
    let assert = run_blocking(cmd).await;
    let out = String::from_utf8(assert.success().get_output().stdout.clone()).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&out).expect("stdout is one JSON doc");
    let rows = parsed.as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0]["name"], "networking");
    assert_eq!(rows[0]["current_run_status"], "applied");
    assert_eq!(rows[0]["vcs_repo"], "acme/networking");
    assert_eq!(rows[2]["selected"], true);
}
