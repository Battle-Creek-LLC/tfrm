//! J3.2: `runs discard` / `runs cancel` — action gates, cross-suggestions,
//! comment bodies, and the force-cancel cooldown gate.

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::json;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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

/// Mount a run whose actions carry the given flags.
async fn mount_run(server: &MockServer, run_id: &str, status: &str, actions: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path(format!("/api/v2/runs/{run_id}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": {
                "id": run_id,
                "type": "runs",
                "attributes": {
                    "status": status,
                    "message": "vcs push",
                    "actions": actions,
                },
                "relationships": {
                    "workspace": {"data": {"id": "ws-two", "type": "workspaces"}}
                }
            },
            "included": [
                {"id": "ws-two", "type": "workspaces", "attributes": {"name": "platform"}}
            ]
        })))
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn discard_posts_comment_and_reports_what_was_discarded() {
    let server = MockServer::start().await;
    mount_run(
        &server,
        "run-wait",
        "planned",
        json!({"is-discardable": true, "is-cancelable": false}),
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/api/v2/runs/run-wait/actions/discard"))
        .and(body_json(json!({"comment": "not needed"})))
        .respond_with(ResponseTemplate::new(202))
        .expect(1)
        .mount(&server)
        .await;

    let dir = project(&server.uri());
    let mut cmd = tfrm_in(dir.path());
    cmd.args(["runs", "discard", "run-wait", "-m", "not needed"]);
    run_blocking(cmd).await.success().stdout(
        predicate::str::contains("Discarded run run-wait")
            .and(predicate::str::contains("planned"))
            .and(predicate::str::contains("vcs push")),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn discard_of_in_flight_run_suggests_cancel() {
    let server = MockServer::start().await;
    mount_run(
        &server,
        "run-flight",
        "planning",
        json!({"is-discardable": false, "is-cancelable": true}),
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/api/v2/runs/run-flight/actions/discard"))
        .respond_with(ResponseTemplate::new(202))
        .expect(0)
        .mount(&server)
        .await;

    let dir = project(&server.uri());
    let mut cmd = tfrm_in(dir.path());
    cmd.args(["runs", "discard", "run-flight"]);
    run_blocking(cmd)
        .await
        .code(6)
        .stderr(predicate::str::contains("tfrm runs cancel"));
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_of_awaiting_run_suggests_discard() {
    let server = MockServer::start().await;
    mount_run(
        &server,
        "run-wait",
        "planned",
        json!({"is-discardable": true, "is-cancelable": false}),
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/api/v2/runs/run-wait/actions/cancel"))
        .respond_with(ResponseTemplate::new(202))
        .expect(0)
        .mount(&server)
        .await;

    let dir = project(&server.uri());
    let mut cmd = tfrm_in(dir.path());
    cmd.args(["runs", "cancel", "run-wait"]);
    run_blocking(cmd)
        .await
        .code(6)
        .stderr(predicate::str::contains("tfrm runs discard"));
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_posts_when_cancelable() {
    let server = MockServer::start().await;
    mount_run(
        &server,
        "run-flight",
        "planning",
        json!({"is-cancelable": true}),
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/api/v2/runs/run-flight/actions/cancel"))
        .and(body_json(json!({"comment": "stop"})))
        .respond_with(ResponseTemplate::new(202))
        .expect(1)
        .mount(&server)
        .await;

    let dir = project(&server.uri());
    let mut cmd = tfrm_in(dir.path());
    cmd.args(["runs", "cancel", "run-flight", "-m", "stop"]);
    run_blocking(cmd)
        .await
        .success()
        .stdout(predicate::str::contains("Canceled run run-flight"));
}

#[tokio::test(flavor = "multi_thread")]
async fn force_refused_until_force_cancelable() {
    let server = MockServer::start().await;
    mount_run(
        &server,
        "run-stuck",
        "canceling",
        json!({"is-cancelable": false, "is-force-cancelable": false}),
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/api/v2/runs/run-stuck/actions/force-cancel"))
        .respond_with(ResponseTemplate::new(202))
        .expect(0)
        .mount(&server)
        .await;

    let dir = project(&server.uri());
    let mut cmd = tfrm_in(dir.path());
    cmd.args(["runs", "cancel", "run-stuck", "--force"]);
    run_blocking(cmd)
        .await
        .code(6)
        .stderr(predicate::str::contains("not force-cancelable"));
}

#[tokio::test(flavor = "multi_thread")]
async fn force_posts_force_cancel_when_allowed() {
    let server = MockServer::start().await;
    mount_run(
        &server,
        "run-stuck",
        "canceling",
        json!({"is-cancelable": false, "is-force-cancelable": true}),
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/api/v2/runs/run-stuck/actions/force-cancel"))
        .respond_with(ResponseTemplate::new(202))
        .expect(1)
        .mount(&server)
        .await;

    let dir = project(&server.uri());
    let mut cmd = tfrm_in(dir.path());
    cmd.args(["runs", "cancel", "run-stuck", "--force"]);
    run_blocking(cmd)
        .await
        .success()
        .stdout(predicate::str::contains("Force-canceled run run-stuck"));
}
