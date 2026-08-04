//! Project config and workspace resolution (R2.2, R2.4, R3.2/R3.3).
//!
//! `.tfrm.toml` (committed project config: `org`, `hostname`, `workspace`)
//! is discovered by walking from the working directory to the filesystem
//! root. `.tfrm/local.toml` (user-local selection written by `tfrm
//! workspace select`, gitignore-recommended) is discovered the same way.
//! Workspace precedence: `-w` flag > selection > config, with the winning
//! source tracked for `workspace current`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

pub const DEFAULT_HOSTNAME: &str = "app.terraform.io";

/// Contents of a discovered `.tfrm.toml`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ProjectConfig {
    pub org: Option<String>,
    pub hostname: Option<String>,
    pub workspace: Option<String>,
}

/// Contents of `.tfrm/local.toml`.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct LocalState {
    workspace: Option<String>,
}

/// Which source produced the resolved workspace (R3.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceSource {
    /// `-w/--workspace` flag.
    Flag,
    /// Selection persisted by `tfrm workspace select` in this file.
    Selection(PathBuf),
    /// `workspace` key in this `.tfrm.toml`.
    Config(PathBuf),
}

impl std::fmt::Display for WorkspaceSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkspaceSource::Flag => write!(f, "-w/--workspace flag"),
            WorkspaceSource::Selection(p) => write!(f, "selection ({})", p.display()),
            WorkspaceSource::Config(p) => write!(f, "config ({})", p.display()),
        }
    }
}

/// Discovered project context: config file, local selection, and where to
/// write a new selection.
#[derive(Debug, Default)]
pub struct Context {
    config: Option<(ProjectConfig, PathBuf)>,
    selection: Option<(String, PathBuf)>,
    /// Directory that anchors `.tfrm/local.toml` writes: beside the
    /// discovered `.tfrm.toml`, else beside an existing `.tfrm/`, else the
    /// starting directory.
    write_root: PathBuf,
}

impl Context {
    /// Walk from `start` up to the filesystem root, picking the nearest
    /// `.tfrm.toml` and the nearest `.tfrm/local.toml` independently.
    pub fn discover(start: &Path) -> Result<Self> {
        let mut config = None;
        let mut selection_file = None;
        for dir in start.ancestors() {
            if config.is_none() {
                let candidate = dir.join(".tfrm.toml");
                if candidate.is_file() {
                    let text = std::fs::read_to_string(&candidate).map_err(|e| {
                        Error::Other(format!("cannot read {}: {e}", candidate.display()))
                    })?;
                    let parsed: ProjectConfig = toml::from_str(&text).map_err(|e| {
                        Error::Usage(format!("invalid {}: {e}", candidate.display()))
                    })?;
                    config = Some((parsed, candidate));
                }
            }
            if selection_file.is_none() {
                let candidate = dir.join(".tfrm").join("local.toml");
                if candidate.is_file() {
                    selection_file = Some(candidate);
                }
            }
            if config.is_some() && selection_file.is_some() {
                break;
            }
        }

        let selection = match &selection_file {
            Some(path) => {
                let text = std::fs::read_to_string(path)
                    .map_err(|e| Error::Other(format!("cannot read {}: {e}", path.display())))?;
                let parsed: LocalState = toml::from_str(&text)
                    .map_err(|e| Error::Usage(format!("invalid {}: {e}", path.display())))?;
                parsed.workspace.map(|ws| (ws, path.clone()))
            }
            None => None,
        };

        let write_root = config
            .as_ref()
            .and_then(|(_, p)| p.parent().map(Path::to_path_buf))
            .or_else(|| {
                selection_file
                    .as_ref()
                    .and_then(|p| p.parent().and_then(Path::parent).map(Path::to_path_buf))
            })
            .unwrap_or_else(|| start.to_path_buf());

        Ok(Context {
            config,
            selection,
            write_root,
        })
    }

    /// Resolve the workspace per R2.4; exit-2 error naming all three
    /// sources when none resolves.
    pub fn resolve_workspace(&self, flag: Option<&str>) -> Result<(String, WorkspaceSource)> {
        if let Some(ws) = flag {
            return Ok((ws.to_string(), WorkspaceSource::Flag));
        }
        if let Some((ws, path)) = &self.selection {
            return Ok((ws.clone(), WorkspaceSource::Selection(path.clone())));
        }
        if let Some((cfg, path)) = &self.config {
            if let Some(ws) = &cfg.workspace {
                return Ok((ws.clone(), WorkspaceSource::Config(path.clone())));
            }
        }
        Err(Error::Usage(
            "no workspace resolved: pass -w/--workspace <NAME>, run `tfrm workspace select \
             <NAME>`, or set `workspace` in .tfrm.toml"
                .into(),
        ))
    }

    /// Resolve the org: `--org` flag over `.tfrm.toml` (R2.2).
    pub fn resolve_org(&self, flag: Option<&str>) -> Result<String> {
        if let Some(org) = flag {
            return Ok(org.to_string());
        }
        if let Some((cfg, _)) = &self.config {
            if let Some(org) = &cfg.org {
                return Ok(org.clone());
            }
        }
        Err(Error::Usage(
            "no organization resolved: pass --org <NAME> or set `org` in .tfrm.toml".into(),
        ))
    }

    /// Hostname from `.tfrm.toml`, defaulting to app.terraform.io (R2.2).
    pub fn hostname(&self) -> String {
        self.config
            .as_ref()
            .and_then(|(cfg, _)| cfg.hostname.clone())
            .unwrap_or_else(|| DEFAULT_HOSTNAME.to_string())
    }

    /// Persist a selection to `.tfrm/local.toml` (R3.2). Returns the file
    /// written.
    pub fn select_workspace(&self, name: &str) -> Result<PathBuf> {
        let dir = self.write_root.join(".tfrm");
        std::fs::create_dir_all(&dir)
            .map_err(|e| Error::Other(format!("cannot create {}: {e}", dir.display())))?;
        let path = dir.join("local.toml");
        let state = LocalState {
            workspace: Some(name.to_string()),
        };
        let text = toml::to_string_pretty(&state)
            .map_err(|e| Error::Other(format!("cannot serialize selection: {e}")))?;
        std::fs::write(&path, text)
            .map_err(|e| Error::Other(format!("cannot write {}: {e}", path.display())))?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn flag_beats_selection_and_config() {
        let dir = tempfile::tempdir().unwrap();
        write(
            &dir.path().join(".tfrm.toml"),
            "org = \"acme\"\nworkspace = \"from-config\"\n",
        );
        write(
            &dir.path().join(".tfrm/local.toml"),
            "workspace = \"from-selection\"\n",
        );
        let ctx = Context::discover(dir.path()).unwrap();
        let (ws, source) = ctx.resolve_workspace(Some("from-flag")).unwrap();
        assert_eq!(ws, "from-flag");
        assert_eq!(source, WorkspaceSource::Flag);
    }

    #[test]
    fn selection_beats_config() {
        let dir = tempfile::tempdir().unwrap();
        write(
            &dir.path().join(".tfrm.toml"),
            "workspace = \"from-config\"\n",
        );
        write(
            &dir.path().join(".tfrm/local.toml"),
            "workspace = \"from-selection\"\n",
        );
        let ctx = Context::discover(dir.path()).unwrap();
        let (ws, source) = ctx.resolve_workspace(None).unwrap();
        assert_eq!(ws, "from-selection");
        assert!(matches!(source, WorkspaceSource::Selection(_)));
    }

    #[test]
    fn config_workspace_used_last() {
        let dir = tempfile::tempdir().unwrap();
        write(
            &dir.path().join(".tfrm.toml"),
            "workspace = \"from-config\"\n",
        );
        let ctx = Context::discover(dir.path()).unwrap();
        let (ws, source) = ctx.resolve_workspace(None).unwrap();
        assert_eq!(ws, "from-config");
        assert!(matches!(source, WorkspaceSource::Config(_)));
    }

    #[test]
    fn none_resolves_names_all_three_sources() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Context::discover(dir.path()).unwrap();
        let err = ctx.resolve_workspace(None).unwrap_err();
        assert_eq!(err.exit_code(), 2);
        let msg = err.to_string();
        assert!(msg.contains("-w/--workspace"), "{msg}");
        assert!(msg.contains("workspace select"), "{msg}");
        assert!(msg.contains(".tfrm.toml"), "{msg}");
    }

    #[test]
    fn discovery_walks_to_ancestors() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join(".tfrm.toml"), "org = \"acme\"\n");
        let nested = dir.path().join("a/b/c");
        std::fs::create_dir_all(&nested).unwrap();
        let ctx = Context::discover(&nested).unwrap();
        assert_eq!(ctx.resolve_org(None).unwrap(), "acme");
        assert_eq!(ctx.hostname(), DEFAULT_HOSTNAME);
    }

    #[test]
    fn hostname_from_config_overrides_default() {
        let dir = tempfile::tempdir().unwrap();
        write(
            &dir.path().join(".tfrm.toml"),
            "org = \"acme\"\nhostname = \"tfe.example.com\"\n",
        );
        let ctx = Context::discover(dir.path()).unwrap();
        assert_eq!(ctx.hostname(), "tfe.example.com");
    }

    #[test]
    fn org_flag_overrides_config() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join(".tfrm.toml"), "org = \"acme\"\n");
        let ctx = Context::discover(dir.path()).unwrap();
        assert_eq!(ctx.resolve_org(Some("other")).unwrap(), "other");
    }

    #[test]
    fn missing_org_is_usage_error() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = Context::discover(dir.path()).unwrap();
        let err = ctx.resolve_org(None).unwrap_err();
        assert_eq!(err.exit_code(), 2);
        assert!(err.to_string().contains("--org"), "{err}");
    }

    #[test]
    fn local_toml_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join(".tfrm.toml"), "org = \"acme\"\n");
        let ctx = Context::discover(dir.path()).unwrap();
        let path = ctx.select_workspace("picked-ws").unwrap();
        assert_eq!(path, dir.path().join(".tfrm/local.toml"));

        let ctx2 = Context::discover(dir.path()).unwrap();
        let (ws, source) = ctx2.resolve_workspace(None).unwrap();
        assert_eq!(ws, "picked-ws");
        assert_eq!(source, WorkspaceSource::Selection(path));
    }

    #[test]
    fn selection_written_beside_config_from_a_subdir() {
        // select run from a nested dir persists next to .tfrm.toml, so the
        // whole project shares one selection.
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join(".tfrm.toml"), "org = \"acme\"\n");
        let nested = dir.path().join("modules/net");
        std::fs::create_dir_all(&nested).unwrap();
        let ctx = Context::discover(&nested).unwrap();
        let path = ctx.select_workspace("picked").unwrap();
        assert_eq!(path, dir.path().join(".tfrm/local.toml"));
    }
}
