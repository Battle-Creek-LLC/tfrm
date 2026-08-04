//! J3.1: `runs apply` wiremock scenarios — happy path with comment and
//! polling, not-confirmable, policy handling, 409, errored terminal,
//! and the confirmation prompt.

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::json;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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
    cmd.current_dir(dir)
        .args(["--token", "test-token"])
        .env("TFRM_POLL_INTERVAL_MS", "10");
    cmd
}

async fn run_blocking(mut cmd: Command) -> assert_cmd::assert::Assert {
    tokio::task::spawn_blocking(move || cmd.assert())
        .await
        .unwrap()
}

fn run_body(run_id: &str, status: &str, confirmable: bool) -> serde_json::Value {
    json!({
        "data": {
            "id": run_id,
            "type": "runs",
            "attributes": {
                "status": status,
                "source": "tfe-api",
                "message": "vcs push",
                "actions": {"is-confirmable": confirmable},
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
    })
}

/// Mount GET /runs/:id answering each status in `sequence` once, with the
/// last status repeating forever.
async fn mount_run_sequence(server: &MockServer, run_id: &str, sequence: &[(&str, bool)]) {
    for (i, (status, confirmable)) in sequence.iter().enumerate() {
        let mock = Mock::given(method("GET"))
            .and(path(format!("/api/v2/runs/{run_id}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(run_body(
                run_id,
                status,
                *confirmable,
            )));
        if i + 1 < sequence.len() {
            mock.up_to_n_times(1).mount(server).await;
        } else {
            mock.mount(server).await;
        }
    }
}

async fn mount_policy_checks(server: &MockServer, run_id: &str, checks: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path(format!("/api/v2/runs/{run_id}/policy-checks")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": checks})))
        .mount(server)
        .await;
}

async fn mount_plan(server: &MockServer, run_id: &str) {
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
        .respond_with(ResponseTemplate::new(200).set_body_json(plan_fixture("update.json")))
        .mount(server)
        .await;
}

async fn mount_apply_record(server: &MockServer, run_id: &str) {
    Mock::given(method("GET"))
        .and(path(format!("/api/v2/runs/{run_id}/apply")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": {"id": "apply-1", "type": "applies", "attributes": {
                "log-read-url": format!("{}/presigned/apply-log", server.uri())
            }}
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/presigned/apply-log"))
        .respond_with(ResponseTemplate::new(200).set_body_string("Applying... done.\n"))
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn happy_path_applies_with_comment_and_polls_to_applied() {
    let server = MockServer::start().await;
    // initial fetch: planned+confirmable; then poll: applying, applied.
    mount_run_sequence(
        &server,
        "run-ok",
        &[("planned", true), ("applying", false), ("applied", false)],
    )
    .await;
    mount_policy_checks(&server, "run-ok", json!([])).await;
    mount_plan(&server, "run-ok").await;
    mount_apply_record(&server, "run-ok").await;
    Mock::given(method("POST"))
        .and(path("/api/v2/runs/run-ok/actions/apply"))
        .and(body_json(json!({"comment": "tfrm e2e"})))
        .respond_with(ResponseTemplate::new(202))
        .expect(1)
        .mount(&server)
        .await;

    let dir = project(&server.uri());
    let mut cmd = tfrm_in(dir.path());
    cmd.args(["runs", "apply", "run-ok", "-m", "tfrm e2e"])
        .write_stdin("platform\n");
    let assert = run_blocking(cmd).await;
    let output = assert.success().get_output().clone();
    let out = String::from_utf8(output.stdout).unwrap();
    let err = String::from_utf8(output.stderr).unwrap();
    assert!(out.contains("Run run-ok applied."), "{out}");
    assert!(
        out.contains("Applying... done."),
        "apply log streamed: {out}"
    );
    assert!(err.contains("1 to change"), "summary shown: {err}");
    assert!(err.contains("feedbeef12345678"), "commit shown: {err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_workspace_name_aborts_without_posting() {
    let server = MockServer::start().await;
    mount_run_sequence(&server, "run-ok", &[("planned", true)]).await;
    mount_policy_checks(&server, "run-ok", json!([])).await;
    mount_plan(&server, "run-ok").await;
    Mock::given(method("POST"))
        .and(path("/api/v2/runs/run-ok/actions/apply"))
        .respond_with(ResponseTemplate::new(202))
        .expect(0) // nothing may be POSTed on abort
        .mount(&server)
        .await;

    let dir = project(&server.uri());
    let mut cmd = tfrm_in(dir.path());
    cmd.args(["runs", "apply", "run-ok"])
        .write_stdin("not-the-workspace\n");
    run_blocking(cmd)
        .await
        .code(1)
        .stderr(predicate::str::contains("apply aborted"));
}

#[tokio::test(flavor = "multi_thread")]
async fn not_confirmable_exits_6_with_reason() {
    let server = MockServer::start().await;
    mount_run_sequence(&server, "run-done", &[("applied", false)]).await;
    mount_policy_checks(&server, "run-done", json!([])).await;

    let dir = project(&server.uri());
    let mut cmd = tfrm_in(dir.path());
    cmd.args(["runs", "apply", "run-done", "--auto-approve"]);
    run_blocking(cmd).await.code(6).stderr(
        predicate::str::contains("not confirmable")
            .and(predicate::str::contains("applied"))
            .and(predicate::str::contains("already finished")),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn soft_mandatory_without_flag_exits_6() {
    let server = MockServer::start().await;
    mount_run_sequence(&server, "run-pol", &[("policy_override", false)]).await;
    mount_policy_checks(
        &server,
        "run-pol",
        json!([{"id": "polchk-1", "type": "policy-checks", "attributes": {
            "status": "soft_failed", "permissions": {"can-override": true}
        }}]),
    )
    .await;

    let dir = project(&server.uri());
    let mut cmd = tfrm_in(dir.path());
    cmd.args(["runs", "apply", "run-pol", "--auto-approve"]);
    run_blocking(cmd)
        .await
        .code(6)
        .stderr(predicate::str::contains("--override-policy"));
}

#[tokio::test(flavor = "multi_thread")]
async fn override_policy_posts_empty_body_then_applies() {
    let server = MockServer::start().await;
    // Sequence: policy_override (initial) → planned+confirmable after
    // override → applied after apply POST.
    mount_run_sequence(
        &server,
        "run-pol",
        &[
            ("policy_override", false),
            ("planned", true),
            ("applied", false),
        ],
    )
    .await;
    mount_policy_checks(
        &server,
        "run-pol",
        json!([{"id": "polchk-1", "type": "policy-checks", "attributes": {
            "status": "soft_failed", "permissions": {"can-override": true}
        }}]),
    )
    .await;
    mount_plan(&server, "run-pol").await;
    mount_apply_record(&server, "run-pol").await;
    // Override POST must carry an empty body (the endpoint takes no comment).
    Mock::given(method("POST"))
        .and(path("/api/v2/policy-checks/polchk-1/actions/override"))
        .and(NoBody)
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": {"id": "polchk-1", "type": "policy-checks",
                      "attributes": {"status": "overridden"}}
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v2/runs/run-pol/actions/apply"))
        .respond_with(ResponseTemplate::new(202))
        .expect(1)
        .mount(&server)
        .await;

    let dir = project(&server.uri());
    let mut cmd = tfrm_in(dir.path());
    cmd.args([
        "runs",
        "apply",
        "run-pol",
        "--override-policy",
        "--auto-approve",
    ]);
    run_blocking(cmd)
        .await
        .success()
        .stderr(predicate::str::contains("policy check is being overridden"));
}

/// Matches requests with an empty body.
struct NoBody;

impl wiremock::Match for NoBody {
    fn matches(&self, request: &wiremock::Request) -> bool {
        request.body.is_empty()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn hard_mandatory_failure_is_final_even_with_flag() {
    let server = MockServer::start().await;
    mount_run_sequence(&server, "run-hard", &[("policy_override", false)]).await;
    mount_policy_checks(
        &server,
        "run-hard",
        json!([{"id": "polchk-2", "type": "policy-checks", "attributes": {
            "status": "hard_failed", "permissions": {"can-override": true}
        }}]),
    )
    .await;

    let dir = project(&server.uri());
    let mut cmd = tfrm_in(dir.path());
    cmd.args([
        "runs",
        "apply",
        "run-hard",
        "--override-policy",
        "--auto-approve",
    ]);
    run_blocking(cmd)
        .await
        .code(6)
        .stderr(predicate::str::contains("hard-mandatory"));
}

#[tokio::test(flavor = "multi_thread")]
async fn conflict_409_on_apply_exits_6_with_detail() {
    let server = MockServer::start().await;
    mount_run_sequence(&server, "run-race", &[("planned", true)]).await;
    mount_policy_checks(&server, "run-race", json!([])).await;
    mount_plan(&server, "run-race").await;
    Mock::given(method("POST"))
        .and(path("/api/v2/runs/run-race/actions/apply"))
        .respond_with(ResponseTemplate::new(409).set_body_json(json!({
            "errors": [{"status": "409", "title": "transition not allowed",
                         "detail": "the run is no longer confirmable"}]
        })))
        .mount(&server)
        .await;

    let dir = project(&server.uri());
    let mut cmd = tfrm_in(dir.path());
    cmd.args(["runs", "apply", "run-race", "--auto-approve"]);
    run_blocking(cmd)
        .await
        .code(6)
        .stderr(predicate::str::contains("no longer confirmable"));
}

#[tokio::test(flavor = "multi_thread")]
async fn errored_terminal_exits_1() {
    let server = MockServer::start().await;
    mount_run_sequence(
        &server,
        "run-bad",
        &[("planned", true), ("applying", false), ("errored", false)],
    )
    .await;
    mount_policy_checks(&server, "run-bad", json!([])).await;
    mount_plan(&server, "run-bad").await;
    mount_apply_record(&server, "run-bad").await;
    Mock::given(method("POST"))
        .and(path("/api/v2/runs/run-bad/actions/apply"))
        .respond_with(ResponseTemplate::new(202))
        .mount(&server)
        .await;

    let dir = project(&server.uri());
    let mut cmd = tfrm_in(dir.path());
    cmd.args(["runs", "apply", "run-bad", "--auto-approve"]);
    run_blocking(cmd)
        .await
        .code(1)
        .stderr(predicate::str::contains("errored"));
}

/// R7.5: 403 on the apply POST names the missing write permission.
#[tokio::test(flavor = "multi_thread")]
async fn forbidden_apply_names_write_permission() {
    let server = MockServer::start().await;
    mount_run_sequence(&server, "run-ro", &[("planned", true)]).await;
    mount_policy_checks(&server, "run-ro", json!([])).await;
    mount_plan(&server, "run-ro").await;
    Mock::given(method("POST"))
        .and(path("/api/v2/runs/run-ro/actions/apply"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "errors": [{"status": "403", "title": "forbidden"}]
        })))
        .mount(&server)
        .await;

    let dir = project(&server.uri());
    let mut cmd = tfrm_in(dir.path());
    cmd.args(["runs", "apply", "run-ro", "--auto-approve"]);
    run_blocking(cmd)
        .await
        .code(3)
        .stderr(predicate::str::contains("\"write\""));
}
