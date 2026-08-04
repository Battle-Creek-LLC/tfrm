//! J1.1 verify: `tfrm auth-debug` reports which credential source resolved
//! (env, file, or neither) without printing the token.

use assert_cmd::Command;
use predicates::prelude::*;

fn tfrm_isolated(home: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("tfrm").unwrap();
    // Isolate from the real environment: no inherited TF_TOKEN_* or HOME.
    cmd.env_clear().env("HOME", home);
    cmd
}

#[test]
fn auth_debug_reports_env_source() {
    let home = tempfile::tempdir().unwrap();
    tfrm_isolated(home.path())
        .env("TF_TOKEN_app_terraform_io", "secret-env-token")
        .arg("auth-debug")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("environment variable TF_TOKEN_app_terraform_io")
                .and(predicate::str::contains("secret-env-token").not()),
        );
}

#[test]
fn auth_debug_reports_file_source() {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(
        home.path().join(".terraformrc"),
        "credentials \"app.terraform.io\" {\n  token = \"secret-file-token\"\n}\n",
    )
    .unwrap();
    tfrm_isolated(home.path())
        .arg("auth-debug")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("credentials file")
                .and(predicate::str::contains(".terraformrc"))
                .and(predicate::str::contains("secret-file-token").not()),
        );
}

#[test]
fn auth_debug_without_credentials_exits_3_with_login_hint() {
    let home = tempfile::tempdir().unwrap();
    tfrm_isolated(home.path())
        .arg("auth-debug")
        .assert()
        .code(3)
        .stderr(predicate::str::contains("tfrm login"));
}

#[test]
fn auth_debug_reports_flag_source() {
    let home = tempfile::tempdir().unwrap();
    tfrm_isolated(home.path())
        .args(["auth-debug", "--token", "secret-flag-token"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("--token flag")
                .and(predicate::str::contains("secret-flag-token").not()),
        );
}
