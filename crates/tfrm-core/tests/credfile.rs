//! J4.2: credential store round-trips — foreign-host preservation, 0600
//! mode, targeted removal, and self-interop with the J1.1 reader.

use tfrm_core::credentials::{CredentialLookup, CredentialSource};
use tfrm_core::credfile;

fn store_path(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path().join(".terraform.d/credentials.tfrc.json")
}

#[test]
fn store_preserves_existing_foreign_host_entries() {
    let dir = tempfile::tempdir().unwrap();
    let path = store_path(&dir);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        r#"{"credentials": {"tfe.example.com": {"token": "foreign-token"}}}"#,
    )
    .unwrap();

    credfile::store(&path, "app.terraform.io", "new-token").unwrap();

    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        doc["credentials"]["tfe.example.com"]["token"],
        "foreign-token"
    );
    assert_eq!(doc["credentials"]["app.terraform.io"]["token"], "new-token");
}

#[cfg(unix)]
#[test]
fn store_creates_file_with_mode_0600() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = store_path(&dir);
    credfile::store(&path, "app.terraform.io", "tok").unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600, "mode was {mode:o}");
}

#[test]
fn remove_deletes_only_the_target_host() {
    let dir = tempfile::tempdir().unwrap();
    let path = store_path(&dir);
    credfile::store(&path, "app.terraform.io", "tok-a").unwrap();
    credfile::store(&path, "tfe.example.com", "tok-b").unwrap();

    assert!(credfile::remove(&path, "app.terraform.io").unwrap());

    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert!(doc["credentials"].get("app.terraform.io").is_none());
    assert_eq!(doc["credentials"]["tfe.example.com"]["token"], "tok-b");
}

#[test]
fn remove_of_absent_host_is_a_reported_no_op() {
    let dir = tempfile::tempdir().unwrap();
    let path = store_path(&dir);
    // Missing file entirely.
    assert!(!credfile::remove(&path, "app.terraform.io").unwrap());
    // File exists but host absent.
    credfile::store(&path, "tfe.example.com", "tok").unwrap();
    assert!(!credfile::remove(&path, "app.terraform.io").unwrap());
}

/// The file our writer produces must resolve through the J1.1 reader —
/// tfrm's own login and lookup stay interoperable (and, by schema,
/// interoperable with terraform's).
#[test]
fn written_file_parses_with_the_credential_reader() {
    let dir = tempfile::tempdir().unwrap();
    let path = store_path(&dir);
    credfile::store(&path, "app.terraform.io", "self-interop-token").unwrap();

    let lookup =
        CredentialLookup::with_sources(vec![], None, None, Some(dir.path().join(".terraform.d")));
    let cred = lookup.resolve("app.terraform.io", None).unwrap();
    assert_eq!(cred.token, "self-interop-token");
    assert_eq!(cred.source, CredentialSource::File(path));
}

/// Hosts are stored in normalized (punycode, lowercase) form so lookups
/// by either spelling match, as terraform's svchost normalization does.
#[test]
fn stored_hosts_are_normalized() {
    let dir = tempfile::tempdir().unwrap();
    let path = store_path(&dir);
    credfile::store(&path, "TFE.Example.COM", "tok").unwrap();
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(doc["credentials"]["tfe.example.com"]["token"], "tok");
    assert!(credfile::remove(&path, "tfe.example.com").unwrap());
}
