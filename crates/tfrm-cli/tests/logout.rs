//! J4.2: `tfrm logout` end-to-end — removal, absent-host no-op, and the
//! exit-3 login hint once the token is gone.

use assert_cmd::Command;
use predicates::prelude::*;

fn tfrm_home(home: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("tfrm").unwrap();
    cmd.env_clear().env("HOME", home);
    cmd
}

fn write_credentials(home: &std::path::Path, hosts: &[(&str, &str)]) {
    let dir = home.join(".terraform.d");
    std::fs::create_dir_all(&dir).unwrap();
    let entries: serde_json::Map<String, serde_json::Value> = hosts
        .iter()
        .map(|(host, token)| (host.to_string(), serde_json::json!({"token": token})))
        .collect();
    std::fs::write(
        dir.join("credentials.tfrc.json"),
        serde_json::json!({"credentials": entries}).to_string(),
    )
    .unwrap();
}

#[test]
fn logout_removes_only_the_target_host() {
    let home = tempfile::tempdir().unwrap();
    write_credentials(
        home.path(),
        &[("app.terraform.io", "tok-a"), ("tfe.example.com", "tok-b")],
    );

    tfrm_home(home.path())
        .arg("logout")
        .assert()
        .success()
        .stdout(predicate::str::contains("Logged out of app.terraform.io"));

    let doc: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(home.path().join(".terraform.d/credentials.tfrc.json")).unwrap(),
    )
    .unwrap();
    assert!(doc["credentials"].get("app.terraform.io").is_none());
    assert_eq!(doc["credentials"]["tfe.example.com"]["token"], "tok-b");
}

#[test]
fn logout_of_absent_host_exits_0_with_note() {
    let home = tempfile::tempdir().unwrap();
    tfrm_home(home.path())
        .arg("logout")
        .assert()
        .success()
        .stdout(predicate::str::contains("nothing to do"));
}

/// After logout, an API command exits 3 with the login hint again.
#[test]
fn workspace_list_after_logout_exits_3_with_hint() {
    let home = tempfile::tempdir().unwrap();
    write_credentials(home.path(), &[("app.terraform.io", "tok-a")]);

    tfrm_home(home.path()).arg("logout").assert().success();

    let project = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join(".tfrm.toml"), "org = \"acme\"\n").unwrap();
    tfrm_home(home.path())
        .current_dir(project.path())
        .args(["workspace", "list"])
        .assert()
        .code(3)
        .stderr(predicate::str::contains("tfrm login"));
}
