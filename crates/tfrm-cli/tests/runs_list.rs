//! J2.2: `runs list` — one request with the configuration-version
//! include (no N+1), commit SHA and source columns, confirmable
//! indicator, --status → filter[status].

use assert_cmd::Command;
use serde_json::json;
use wiremock::matchers::{method, path, query_param};
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

async fn mount_workspace(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/api/v2/organizations/acme/workspaces/platform"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": {"id": "ws-two", "type": "workspaces", "attributes": {"name": "platform"}}
        })))
        .mount(server)
        .await;
}

fn run_doc(id: &str, status: &str, confirmable: bool, cv: &str) -> serde_json::Value {
    json!({
        "id": id,
        "type": "runs",
        "attributes": {
            "status": status,
            "created-at": "2026-08-03T10:00:00Z",
            "message": format!("message for {id}"),
            "source": "tfe-api",
            "actions": {"is-confirmable": confirmable},
        },
        "relationships": {
            "configuration-version": {"data": {"id": cv, "type": "configuration-versions"}}
        }
    })
}

/// The included chain: run → configuration-version → ingress-attributes.
fn included(cv: &str, ia: &str, sha: &str) -> Vec<serde_json::Value> {
    vec![
        json!({
            "id": cv,
            "type": "configuration-versions",
            "relationships": {
                "ingress-attributes": {"data": {"id": ia, "type": "ingress-attributes"}}
            }
        }),
        json!({
            "id": ia,
            "type": "ingress-attributes",
            "attributes": {"commit-sha": sha}
        }),
    ]
}

#[tokio::test(flavor = "multi_thread")]
async fn list_shows_commit_source_and_confirmable_in_one_request() {
    let server = MockServer::start().await;
    mount_workspace(&server).await;
    let mut inc = included("cv-1", "ia-1", "aaaa1111bbbb2222");
    inc.extend(included("cv-2", "ia-2", "cccc3333dddd4444"));
    Mock::given(method("GET"))
        .and(path("/api/v2/workspaces/ws-two/runs"))
        .and(query_param(
            "include",
            "configuration_version.ingress_attributes",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [
                run_doc("run-new", "planned", true, "cv-1"),
                run_doc("run-old", "applied", false, "cv-2"),
            ],
            "included": inc,
            "meta": {"pagination": {"current-page": 1, "next-page": null, "total-pages": 1}}
        })))
        // exactly one request — the include chain must prevent any N+1
        .expect(1)
        .mount(&server)
        .await;

    let dir = project(&server.uri());
    let mut cmd = tfrm_in(dir.path());
    cmd.args(["runs", "list"]);
    let assert = run_blocking(cmd).await;
    let out = String::from_utf8(assert.success().get_output().stdout.clone()).unwrap();

    for header in ["RUN ID", "STATUS", "CREATED", "COMMIT", "SOURCE", "MESSAGE"] {
        assert!(out.contains(header), "missing header {header}:\n{out}");
    }
    assert!(out.contains("aaaa1111"), "commit sha column:\n{out}");
    assert!(out.contains("tfe-api"), "source column:\n{out}");
    // confirmable marker on run-new only
    let marked: Vec<&str> = out
        .lines()
        .filter(|l| l.starts_with('>') && l.contains("run-"))
        .collect();
    assert_eq!(marked.len(), 1, "{out}");
    assert!(marked[0].contains("run-new"), "{out}");
}

/// A VCS run message carries the full multi-line commit body; the table
/// must render only the subject line, never spilling the body as extra
/// lines after the row.
#[tokio::test(flavor = "multi_thread")]
async fn multi_line_message_renders_subject_only() {
    let server = MockServer::start().await;
    mount_workspace(&server).await;
    let mut run = run_doc("run-dep", "pending", false, "cv-1");
    run["attributes"]["message"] = serde_json::Value::String(
        "Bump provider from 2.66.0 to 2.97.0 (#41)\n\nBumps [provider](https://example.com).\n\
         - [Release notes](https://example.com/releases)\n\n---\nupdated-dependencies:\n\
         - dependency-name: provider"
            .into(),
    );
    Mock::given(method("GET"))
        .and(path("/api/v2/workspaces/ws-two/runs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [run],
            "included": included("cv-1", "ia-1", "aaaa1111bbbb2222"),
            "meta": {"pagination": {"current-page": 1, "next-page": null, "total-pages": 1}}
        })))
        .mount(&server)
        .await;

    let dir = project(&server.uri());
    let mut cmd = tfrm_in(dir.path());
    cmd.args(["runs", "list"]);
    let assert = run_blocking(cmd).await;
    let out = String::from_utf8(assert.success().get_output().stdout.clone()).unwrap();

    assert!(
        out.contains("Bump provider from 2.66.0 to 2.97.0 (#41)"),
        "{out}"
    );
    assert!(!out.contains("updated-dependencies"), "body leaked:\n{out}");
    assert!(!out.contains("Release notes"), "body leaked:\n{out}");
    // Every non-empty output line is a header or a run row — nothing
    // spills outside the table.
    for line in out.lines().filter(|l| !l.trim().is_empty()) {
        assert!(
            line.contains("RUN ID") || line.contains("run-dep"),
            "stray line outside the table: {line:?}\n{out}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn status_flag_maps_to_filter_status() {
    let server = MockServer::start().await;
    mount_workspace(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v2/workspaces/ws-two/runs"))
        .and(query_param("filter[status]", "planned"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [run_doc("run-new", "planned", true, "cv-1")],
            "included": included("cv-1", "ia-1", "aaaa1111bbbb2222"),
            "meta": {"pagination": {"current-page": 1, "next-page": null, "total-pages": 1}}
        })))
        .expect(1)
        .mount(&server)
        .await;

    let dir = project(&server.uri());
    let mut cmd = tfrm_in(dir.path());
    cmd.args(["runs", "list", "--status", "planned"]);
    let assert = run_blocking(cmd).await;
    let out = String::from_utf8(assert.success().get_output().stdout.clone()).unwrap();
    assert!(out.contains("run-new"), "{out}");
}

#[tokio::test(flavor = "multi_thread")]
async fn json_format_carries_all_columns() {
    let server = MockServer::start().await;
    mount_workspace(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v2/workspaces/ws-two/runs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [run_doc("run-new", "planned", true, "cv-1")],
            "included": included("cv-1", "ia-1", "aaaa1111bbbb2222"),
            "meta": {"pagination": {"current-page": 1, "next-page": null, "total-pages": 1}}
        })))
        .mount(&server)
        .await;

    let dir = project(&server.uri());
    let mut cmd = tfrm_in(dir.path());
    cmd.args(["runs", "list", "--format", "json"]);
    let assert = run_blocking(cmd).await;
    let out = String::from_utf8(assert.success().get_output().stdout.clone()).unwrap();
    let rows: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(rows[0]["id"], "run-new");
    assert_eq!(rows[0]["status"], "planned");
    assert_eq!(rows[0]["commit_sha"], "aaaa1111bbbb2222");
    assert_eq!(rows[0]["source"], "tfe-api");
    assert_eq!(rows[0]["confirmable"], true);
}
