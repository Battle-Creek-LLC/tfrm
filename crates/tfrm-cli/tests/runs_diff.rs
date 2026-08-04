//! J2.5 end-to-end: `runs diff` — --exit-code, cross-workspace refusal,
//! R6.7 403 behavior, and sentinel absence on real process output.

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SENTINEL: &str = "SENTINEL-DO-NOT-PRINT";

fn plan_fixture(name: &str) -> serde_json::Value {
    let text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/plans")
            .join(name),
    )
    .unwrap();
    serde_json::from_str(&text).unwrap()
}

fn project(server_uri: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(".tfrm.toml"),
        format!("org = \"acme\"\nhostname = \"{server_uri}\"\nworkspace = \"platform\"\n"),
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

async fn mount_run(server: &MockServer, run_id: &str, ws_id: &str) {
    Mock::given(method("GET"))
        .and(path(format!("/api/v2/runs/{run_id}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": {
                "id": run_id,
                "type": "runs",
                "attributes": {"status": "planned", "source": "tfe-api"},
                "relationships": {
                    "workspace": {"data": {"id": ws_id, "type": "workspaces"}}
                }
            },
            "included": []
        })))
        .mount(server)
        .await;
}

async fn mount_plan(server: &MockServer, run_id: &str, fixture_name: &str) {
    let presigned = format!("/presigned/{run_id}");
    Mock::given(method("GET"))
        .and(path(format!("/api/v2/runs/{run_id}/plan/json-output")))
        .respond_with(
            ResponseTemplate::new(307)
                .insert_header("Location", format!("{}{presigned}", server.uri()).as_str()),
        )
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(presigned))
        .respond_with(ResponseTemplate::new(200).set_body_json(plan_fixture(fixture_name)))
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn diff_shows_sensitive_differs_without_values() {
    let server = MockServer::start().await;
    mount_run(&server, "run-a", "ws-two").await;
    mount_run(&server, "run-b", "ws-two").await;
    mount_plan(&server, "run-a", "sensitive.json").await;
    mount_plan(&server, "run-b", "sensitive-differ.json").await;

    let dir = project(&server.uri());
    let mut cmd = tfrm_in(dir.path());
    cmd.args(["runs", "diff", "run-a", "run-b"]);
    let assert = run_blocking(cmd).await;
    let output = assert.success().get_output().clone();
    let out = String::from_utf8(output.stdout).unwrap();
    let err = String::from_utf8(output.stderr).unwrap();
    assert!(out.contains("password: (sensitive — differs)"), "{out}");
    assert!(!out.contains(SENTINEL), "sentinel on stdout:\n{out}");
    assert!(!err.contains(SENTINEL), "sentinel on stderr:\n{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn exit_code_flag_exits_1_on_differences() {
    let server = MockServer::start().await;
    mount_run(&server, "run-a", "ws-two").await;
    mount_run(&server, "run-b", "ws-two").await;
    mount_plan(&server, "run-a", "update.json").await;
    mount_plan(&server, "run-b", "update-b.json").await;

    let dir = project(&server.uri());
    let mut cmd = tfrm_in(dir.path());
    cmd.args(["runs", "diff", "run-a", "run-b", "--exit-code"]);
    run_blocking(cmd).await.code(1);
}

#[tokio::test(flavor = "multi_thread")]
async fn identical_plans_exit_0_even_with_exit_code() {
    let server = MockServer::start().await;
    mount_run(&server, "run-a", "ws-two").await;
    mount_run(&server, "run-b", "ws-two").await;
    mount_plan(&server, "run-a", "update.json").await;
    mount_plan(&server, "run-b", "update.json").await;

    let dir = project(&server.uri());
    let mut cmd = tfrm_in(dir.path());
    cmd.args(["runs", "diff", "run-a", "run-b", "--exit-code"]);
    run_blocking(cmd)
        .await
        .success()
        .stdout(predicate::str::contains("No differences."));
}

#[tokio::test(flavor = "multi_thread")]
async fn cross_workspace_diff_is_refused_without_flag() {
    let server = MockServer::start().await;
    mount_run(&server, "run-a", "ws-one").await;
    mount_run(&server, "run-b", "ws-two").await;

    let dir = project(&server.uri());
    let mut cmd = tfrm_in(dir.path());
    cmd.args(["runs", "diff", "run-a", "run-b"]);
    run_blocking(cmd)
        .await
        .code(2)
        .stderr(predicate::str::contains("--allow-cross-workspace"));
}

#[tokio::test(flavor = "multi_thread")]
async fn cross_workspace_diff_allowed_with_flag() {
    let server = MockServer::start().await;
    mount_run(&server, "run-a", "ws-one").await;
    mount_run(&server, "run-b", "ws-two").await;
    mount_plan(&server, "run-a", "update.json").await;
    mount_plan(&server, "run-b", "update.json").await;

    let dir = project(&server.uri());
    let mut cmd = tfrm_in(dir.path());
    cmd.args(["runs", "diff", "run-a", "run-b", "--allow-cross-workspace"]);
    run_blocking(cmd).await.success();
}

/// R6.7: the summary fallback is not enough for diff — 403 exits 3.
#[tokio::test(flavor = "multi_thread")]
async fn forbidden_plan_json_exits_3_naming_admin() {
    let server = MockServer::start().await;
    mount_run(&server, "run-a", "ws-two").await;
    mount_run(&server, "run-b", "ws-two").await;
    Mock::given(method("GET"))
        .and(path("/api/v2/runs/run-a/plan/json-output"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "errors": [{"status": "403", "title": "forbidden"}]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/runs/run-a/plan"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": {"id": "plan-1", "type": "plans", "attributes": {
                "resource-additions": 1, "resource-changes": 0, "resource-destructions": 0
            }}
        })))
        .mount(&server)
        .await;

    let dir = project(&server.uri());
    let mut cmd = tfrm_in(dir.path());
    cmd.args(["runs", "diff", "run-a", "run-b"]);
    run_blocking(cmd)
        .await
        .code(3)
        .stderr(predicate::str::contains("workspace admin"));
}

#[tokio::test(flavor = "multi_thread")]
async fn against_latest_applied_resolves_b() {
    let server = MockServer::start().await;
    mount_run(&server, "run-a", "ws-two").await;
    mount_run(&server, "run-applied", "ws-two").await;
    mount_plan(&server, "run-a", "update.json").await;
    mount_plan(&server, "run-applied", "update-b.json").await;
    Mock::given(method("GET"))
        .and(path("/api/v2/workspaces/ws-two/runs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{"id": "run-applied", "type": "runs",
                       "attributes": {"status": "applied"}}]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let dir = project(&server.uri());
    let mut cmd = tfrm_in(dir.path());
    cmd.args(["runs", "diff", "run-a", "--against", "latest-applied"]);
    run_blocking(cmd)
        .await
        .success()
        .stdout(predicate::str::contains("Diff run-a -> run-applied"));
}
