//! J4.1: `tfrm login` — discovery gate, PKCE round-trip, state handling
//! on the paste path, and the paste-only fallback when no callback port
//! can be bound.

use assert_cmd::Command;
use base64::Engine;
use predicates::prelude::*;
use serde_json::json;
use sha2::{Digest, Sha256};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn tfrm_login(server_uri: &str, home: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("tfrm").unwrap();
    cmd.env_clear()
        .env("HOME", home)
        .env("TFRM_NO_BROWSER", "1")
        .args(["login", server_uri]);
    cmd
}

async fn run_blocking(mut cmd: Command) -> assert_cmd::assert::Assert {
    tokio::task::spawn_blocking(move || cmd.assert())
        .await
        .unwrap()
}

/// Discovery doc advertising login.v1 with endpoints on the mock server.
/// Port 0 makes the callback listener bind an ephemeral port, so tests
/// never collide.
async fn mount_discovery(server: &MockServer, ports: (u16, u16)) {
    Mock::given(method("GET"))
        .and(path("/.well-known/terraform.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "login.v1": {
                "client": "terraform-cli",
                "grant_types": ["authz_code"],
                "authz": format!("{}/oauth/authorize", server.uri()),
                "token": format!("{}/oauth/token", server.uri()),
                "ports": [ports.0, ports.1],
            }
        })))
        .mount(server)
        .await;
}

async fn mount_token_and_account(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "new-oauth-token",
            "token_type": "bearer"
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/account/details"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": {"id": "user-1", "type": "users",
                      "attributes": {"username": "jane.doe"}}
        })))
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn host_without_login_v1_exits_4() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/.well-known/terraform.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "modules.v1": "/api/registry/v1/modules/"
        })))
        .mount(&server)
        .await;

    let home = tempfile::tempdir().unwrap();
    let cmd = tfrm_login(&server.uri(), home.path());
    run_blocking(cmd)
        .await
        .code(4)
        .stderr(predicate::str::contains("login.v1"));
}

#[tokio::test(flavor = "multi_thread")]
async fn bare_code_path_completes_with_pkce_round_trip() {
    let server = MockServer::start().await;
    mount_discovery(&server, (0, 0)).await;
    mount_token_and_account(&server).await;

    let home = tempfile::tempdir().unwrap();
    let mut cmd = tfrm_login(&server.uri(), home.path());
    cmd.write_stdin("pasted-bare-code\n");
    let assert = run_blocking(cmd).await;
    let output = assert.success().get_output().clone();
    let out = String::from_utf8(output.stdout).unwrap();
    let err = String::from_utf8(output.stderr).unwrap();

    // Account name printed; token never printed anywhere.
    assert!(out.contains("jane.doe"), "{out}");
    assert!(!out.contains("new-oauth-token"), "{out}");
    assert!(!err.contains("new-oauth-token"), "{err}");
    // Bare code → the skipped-state note (R2b.3).
    assert!(err.contains("state check was skipped"), "{err}");

    // Token stored in the terraform-compatible file.
    let stored =
        std::fs::read_to_string(home.path().join(".terraform.d/credentials.tfrc.json")).unwrap();
    assert!(stored.contains("new-oauth-token"), "{stored}");

    // PKCE round-trip: challenge printed in the authorize URL must be the
    // S256 hash of the verifier sent to the token endpoint.
    let authorize_url = err
        .lines()
        .find(|l| l.contains("code_challenge="))
        .expect("authorize URL printed")
        .trim();
    let parsed = url::Url::parse(authorize_url).unwrap();
    let params: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
    let challenge = params.get("code_challenge").unwrap().clone();
    assert!(params.contains_key("state"), "state in authorize URL");

    let requests = server.received_requests().await.unwrap();
    let token_request = requests
        .iter()
        .find(|r| r.url.path() == "/oauth/token")
        .expect("token request sent");
    let form: std::collections::HashMap<String, String> =
        url::form_urlencoded::parse(&token_request.body)
            .into_owned()
            .collect();
    assert_eq!(
        form.get("code").map(String::as_str),
        Some("pasted-bare-code")
    );
    assert_eq!(
        form.get("grant_type").map(String::as_str),
        Some("authorization_code")
    );
    let verifier = form.get("code_verifier").expect("code_verifier sent");
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    assert_eq!(
        challenge,
        engine.encode(Sha256::digest(verifier.as_bytes()))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn pasted_url_with_wrong_state_is_refused() {
    let server = MockServer::start().await;
    mount_discovery(&server, (0, 0)).await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0) // a refused code must never be exchanged
        .mount(&server)
        .await;

    let home = tempfile::tempdir().unwrap();
    let mut cmd = tfrm_login(&server.uri(), home.path());
    cmd.write_stdin("http://localhost:10007/login?code=evil&state=wrong-state\n");
    run_blocking(cmd)
        .await
        .code(3)
        .stderr(predicate::str::contains("state does not match"));
}

#[tokio::test(flavor = "multi_thread")]
async fn port_exhaustion_falls_back_to_paste_only() {
    let server = MockServer::start().await;
    // Occupy a port ourselves so the advertised single-port range is
    // guaranteed exhausted (privileged ports can be bindable in
    // containers, so port 1 is not a reliable failure).
    let blocker = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let taken_port = blocker.local_addr().unwrap().port();
    mount_discovery(&server, (taken_port, taken_port)).await;
    mount_token_and_account(&server).await;

    let home = tempfile::tempdir().unwrap();
    let mut cmd = tfrm_login(&server.uri(), home.path());
    cmd.write_stdin("pasted-bare-code\n");
    let assert = run_blocking(cmd).await;
    let output = assert.success().get_output().clone();
    let err = String::from_utf8(output.stderr).unwrap();
    assert!(
        err.contains("paste the redirect URL or code manually"),
        "{err}"
    );
}
