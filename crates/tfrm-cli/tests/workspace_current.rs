//! J1.3 verify: `tfrm workspace current` names the winning source.

use assert_cmd::Command;
use predicates::prelude::*;

fn tfrm_in(dir: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("tfrm").unwrap();
    cmd.current_dir(dir);
    cmd
}

#[test]
fn current_names_the_config_file_as_source() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(".tfrm.toml"),
        "org = \"acme\"\nworkspace = \"cfg-ws\"\n",
    )
    .unwrap();
    tfrm_in(dir.path())
        .args(["workspace", "current"])
        .assert()
        .success()
        .stdout(predicate::str::contains("cfg-ws").and(predicate::str::contains("config")));
}

#[test]
fn current_names_the_selection_after_select() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(".tfrm.toml"),
        "org = \"acme\"\nworkspace = \"cfg-ws\"\n",
    )
    .unwrap();
    // Simulate a prior `tfrm workspace select` (the API-verified command
    // itself lands in J2.1).
    std::fs::create_dir_all(dir.path().join(".tfrm")).unwrap();
    std::fs::write(
        dir.path().join(".tfrm/local.toml"),
        "workspace = \"picked-ws\"\n",
    )
    .unwrap();
    tfrm_in(dir.path())
        .args(["workspace", "current"])
        .assert()
        .success()
        .stdout(predicate::str::contains("picked-ws").and(predicate::str::contains("selection")));
}

#[test]
fn current_prefers_the_flag() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".tfrm.toml"), "workspace = \"cfg-ws\"\n").unwrap();
    tfrm_in(dir.path())
        .args(["workspace", "current", "-w", "flag-ws"])
        .assert()
        .success()
        .stdout(predicate::str::contains("flag-ws").and(predicate::str::contains("flag")));
}

#[test]
fn current_with_nothing_resolvable_exits_2_naming_sources() {
    let dir = tempfile::tempdir().unwrap();
    tfrm_in(dir.path())
        .args(["workspace", "current"])
        .assert()
        .code(2)
        .stderr(
            predicate::str::contains("-w/--workspace")
                .and(predicate::str::contains("workspace select"))
                .and(predicate::str::contains(".tfrm.toml")),
        );
}
