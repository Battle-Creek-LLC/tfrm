//! J0.1: every §1 subcommand exists and exits 1 with "not implemented".

use assert_cmd::Command;
use predicates::prelude::*;

fn tfrm() -> Command {
    Command::cargo_bin("tfrm").unwrap()
}

#[test]
fn version_prints_and_succeeds() {
    tfrm()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn help_lists_the_command_tree() {
    let top = tfrm().arg("--help").assert().success();
    let out = String::from_utf8(top.get_output().stdout.clone()).unwrap();
    for cmd in ["login", "logout", "workspace", "runs"] {
        assert!(out.contains(cmd), "top-level help missing `{cmd}`:\n{out}");
    }

    let ws = tfrm().args(["workspace", "--help"]).assert().success();
    let out = String::from_utf8(ws.get_output().stdout.clone()).unwrap();
    for cmd in ["list", "select", "current"] {
        assert!(out.contains(cmd), "workspace help missing `{cmd}`:\n{out}");
    }

    let runs = tfrm().args(["runs", "--help"]).assert().success();
    let out = String::from_utf8(runs.get_output().stdout.clone()).unwrap();
    for cmd in ["list", "show", "diff", "apply", "discard", "cancel"] {
        assert!(out.contains(cmd), "runs help missing `{cmd}`:\n{out}");
    }
}
