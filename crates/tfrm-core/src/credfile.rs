//! Read-modify-write of `~/.terraform.d/credentials.tfrc.json` (R2b.5,
//! R2b.6) — the same file terraform reads and writes, so credentials
//! flow both ways. `credentials` blocks in `.terraformrc` are never
//! touched (read-only config, matching terraform).

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::credentials::normalize_host;
use crate::error::{Error, Result};

/// Default store path: `~/.terraform.d/credentials.tfrc.json`.
pub fn default_path() -> Result<PathBuf> {
    dirs::home_dir()
        .map(|home| home.join(".terraform.d/credentials.tfrc.json"))
        .ok_or_else(|| Error::Other("cannot determine the home directory".into()))
}

fn load(path: &Path) -> Result<Value> {
    if !path.exists() {
        return Ok(json!({"credentials": {}}));
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| Error::Other(format!("cannot read {}: {e}", path.display())))?;
    serde_json::from_str(&text)
        .map_err(|e| Error::Other(format!("invalid JSON in {}: {e}", path.display())))
}

fn save(path: &Path, doc: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| Error::Other(format!("cannot create {}: {e}", parent.display())))?;
    }
    let text = serde_json::to_string_pretty(doc)
        .map_err(|e| Error::Other(format!("cannot serialize credentials: {e}")))?;
    std::fs::write(path, text)
        .map_err(|e| Error::Other(format!("cannot write {}: {e}", path.display())))?;
    // R2b.5: the file holds tokens — owner-only.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| Error::Other(format!("cannot chmod {}: {e}", path.display())))?;
    }
    Ok(())
}

/// Store `token` for `host`, preserving every other host's entry.
pub fn store(path: &Path, host: &str, token: &str) -> Result<()> {
    let host =
        normalize_host(host).ok_or_else(|| Error::Usage(format!("invalid hostname: {host}")))?;
    let mut doc = load(path)?;
    if !doc.get("credentials").is_some_and(Value::is_object) {
        doc["credentials"] = json!({});
    }
    doc["credentials"][&host] = json!({"token": token});
    save(path, &doc)
}

/// Remove `host`'s entry (R2b.6). Returns false (with no error) when the
/// host had no entry — logout of an absent host is a no-op.
pub fn remove(path: &Path, host: &str) -> Result<bool> {
    let host =
        normalize_host(host).ok_or_else(|| Error::Usage(format!("invalid hostname: {host}")))?;
    if !path.exists() {
        return Ok(false);
    }
    let mut doc = load(path)?;
    let removed = doc
        .get_mut("credentials")
        .and_then(Value::as_object_mut)
        .and_then(|creds| creds.remove(&host))
        .is_some();
    if removed {
        save(path, &doc)?;
    }
    Ok(removed)
}
