//! J2.4 end-to-end: `runs show` over wiremock — full plan via 307,
//! degraded 403 summary, sentinel invariant on real process output, and
//! the J0.2 no-credentials exit code.

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SENTINEL: &str = "SENTINEL-DO-NOT-PRINT";

fn sensitive_plan() -> serde_json::Value {
    let text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/plans/sensitive.json"),
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

async fn mount_run(server: &MockServer, status: &str) {
    Mock::given(method("GET"))
        .and(path("/api/v2/runs/run-abc123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": {
                "id": "run-abc123",
                "type": "runs",
                "attributes": {
                    "status": status,
                    "source": "tfe-api",
                    "message": "change the db password",
                    "actions": {"is-confirmable": true},
                },
                "relationships": {
                    "workspace": {"data": {"id": "ws-two", "type": "workspaces"}},
                    "configuration-version": {"data": {"id": "cv-1", "type": "configuration-versions"}}
                }
            },
            "included": [
                {"id": "ws-two", "type": "workspaces", "attributes": {"name": "platform"}},
                {
                    "id": "cv-1",
                    "type": "configuration-versions",
                    "relationships": {"ingress-attributes": {"data": {"id": "ia-1", "type": "ingress-attributes"}}}
                },
                {"id": "ia-1", "type": "ingress-attributes", "attributes": {"commit-sha": "feedbeef12345678"}}
            ]
        })))
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn show_renders_full_plan_with_redaction() {
    let server = MockServer::start().await;
    mount_run(&server, "planned").await;
    Mock::given(method("GET"))
        .and(path("/api/v2/runs/run-abc123/plan/json-output"))
        .respond_with(ResponseTemplate::new(307).insert_header(
            "Location",
            format!("{}/presigned/plan", server.uri()).as_str(),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/presigned/plan"))
        .respond_with(ResponseTemplate::new(200).set_body_json(sensitive_plan()))
        .mount(&server)
        .await;

    let dir = project(&server.uri());
    let mut cmd = tfrm_in(dir.path());
    cmd.args(["runs", "show", "run-abc123"]);
    let assert = run_blocking(cmd).await;
    let output = assert.success().get_output().clone();
    let out = String::from_utf8(output.stdout).unwrap();
    let err = String::from_utf8(output.stderr).unwrap();

    assert!(out.contains("Run run-abc123"), "{out}");
    assert!(out.contains("Workspace: platform"), "{out}");
    assert!(out.contains("Commit:    feedbeef12345678"), "{out}");
    assert!(
        out.contains("password: (sensitive) -> (sensitive)"),
        "{out}"
    );
    // The redaction invariant, on the real process streams.
    assert!(!out.contains(SENTINEL), "sentinel on stdout:\n{out}");
    assert!(!err.contains(SENTINEL), "sentinel on stderr:\n{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn show_json_is_redacted_and_parseable() {
    let server = MockServer::start().await;
    mount_run(&server, "planned").await;
    Mock::given(method("GET"))
        .and(path("/api/v2/runs/run-abc123/plan/json-output"))
        .respond_with(ResponseTemplate::new(307).insert_header(
            "Location",
            format!("{}/presigned/plan", server.uri()).as_str(),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/presigned/plan"))
        .respond_with(ResponseTemplate::new(200).set_body_json(sensitive_plan()))
        .mount(&server)
        .await;

    let dir = project(&server.uri());
    let mut cmd = tfrm_in(dir.path());
    cmd.args(["runs", "show", "run-abc123", "--format", "json"]);
    let assert = run_blocking(cmd).await;
    let out = String::from_utf8(assert.success().get_output().stdout.clone()).unwrap();
    assert!(!out.contains(SENTINEL), "sentinel in JSON output:\n{out}");
    let doc: serde_json::Value = serde_json::from_str(&out).expect("stdout parses as one JSON doc");
    assert_eq!(doc["run"]["id"], "run-abc123");
    assert_eq!(
        doc["resource_changes"][0]["attributes"][1]["after"],
        "(sensitive)"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn show_degrades_on_403_and_exits_0() {
    let server = MockServer::start().await;
    mount_run(&server, "planned").await;
    Mock::given(method("GET"))
        .and(path("/api/v2/runs/run-abc123/plan/json-output"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "errors": [{"status": "403", "title": "forbidden"}]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/runs/run-abc123/plan"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": {"id": "plan-1", "type": "plans", "attributes": {
                "resource-additions": 2, "resource-changes": 0, "resource-destructions": 1
            }}
        })))
        .mount(&server)
        .await;

    let dir = project(&server.uri());
    let mut cmd = tfrm_in(dir.path());
    cmd.args(["runs", "show", "run-abc123"]);
    let assert = run_blocking(cmd).await;
    assert
        .success()
        .stdout(predicate::str::contains(
            "Plan: 2 to add, 0 to change, 1 to destroy. (summary only)",
        ))
        .stderr(predicate::str::contains("workspace admin"));
}

/// J0.2 verify: no credentials anywhere → exit 3 with the R2.1 hint.
#[tokio::test(flavor = "multi_thread")]
async fn show_without_credentials_exits_3_with_login_hint() {
    let home = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".tfrm.toml"), "org = \"acme\"\n").unwrap();
    let mut cmd = Command::cargo_bin("tfrm").unwrap();
    cmd.current_dir(dir.path())
        .env_clear()
        .env("HOME", home.path())
        .args(["runs", "show", "run-x"]);
    let assert = run_blocking(cmd).await;
    assert
        .code(3)
        .stderr(predicate::str::contains("tfrm login"));
}
