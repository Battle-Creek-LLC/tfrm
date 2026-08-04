//! Terraform-compatible credential resolution (R2.1, R2.1a).
//!
//! Mirrors terraform's `internal/command/cliconfig`: `TF_TOKEN_<host>` env
//! vars (with terraform's host mangling), `credentials "<host>"` blocks from
//! the CLI config file, and `credentials.tfrc.json` from the config dir.
//! Precedence: flag > env > file. `credentials_helper` blocks are recognized
//! but never executed in v0.1 (R2.1a).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Where a token came from — reported by `auth-debug` and `workspace
/// current`-style sources; never carries the token itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialSource {
    /// `--token` flag.
    Flag,
    /// `TF_TOKEN_<host>` environment variable (the actual var name).
    Env(String),
    /// A credentials block or entry in this file.
    File(PathBuf),
}

impl std::fmt::Display for CredentialSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CredentialSource::Flag => write!(f, "--token flag"),
            CredentialSource::Env(var) => write!(f, "environment variable {var}"),
            CredentialSource::File(path) => write!(f, "credentials file {}", path.display()),
        }
    }
}

/// A resolved token plus its provenance.
#[derive(Debug, Clone)]
pub struct Credential {
    pub token: String,
    pub source: CredentialSource,
}

/// Normalize a hostname to its lowercase punycode (ACE) comparison form,
/// tolerating both Unicode ("café.fr") and pre-punycoded ("xn--caf-dma.fr")
/// input, like terraform's svchost.ForComparison.
pub fn normalize_host(raw: &str) -> Option<String> {
    let ascii = idna::domain_to_ascii(raw.trim()).ok()?;
    if ascii.is_empty() {
        None
    } else {
        Some(ascii)
    }
}

/// Undo terraform's env-var host mangling: `__` → `-` first (unambiguous
/// because hyphens cannot start or end a DNS label), then `_` → `.`.
fn unmangle_env_host(raw: &str) -> String {
    raw.replace("__", "-").replace('_', ".")
}

/// Credential lookup with every input injectable for tests. `from_os` wires
/// the real environment.
#[derive(Debug, Default)]
pub struct CredentialLookup {
    /// Environment snapshot (name, value), scanned for `TF_TOKEN_*`.
    env: Vec<(String, String)>,
    /// `TF_CLI_CONFIG_FILE` override. When set, the config dir is skipped —
    /// terraform treats the override as "ignore the default config files".
    cli_config_override: Option<PathBuf>,
    /// Default CLI config file (`~/.terraformrc`; Windows `%APPDATA%\terraform.rc`).
    default_config_file: Option<PathBuf>,
    /// Config dir (`~/.terraform.d`), scanned for `*.tfrc` / `*.tfrc.json`
    /// including `credentials.tfrc.json`.
    config_dir: Option<PathBuf>,
}

impl CredentialLookup {
    pub fn from_os() -> Self {
        let home = dirs::home_dir();
        #[cfg(windows)]
        let (default_config_file, config_dir) = {
            let appdata = std::env::var_os("APPDATA").map(PathBuf::from);
            (
                appdata.as_ref().map(|d| d.join("terraform.rc")),
                appdata.as_ref().map(|d| d.join("terraform.d")),
            )
        };
        #[cfg(not(windows))]
        let (default_config_file, config_dir) = (
            home.as_ref().map(|h| h.join(".terraformrc")),
            home.as_ref().map(|h| h.join(".terraform.d")),
        );

        CredentialLookup {
            env: std::env::vars().collect(),
            cli_config_override: std::env::var_os("TF_CLI_CONFIG_FILE").map(PathBuf::from),
            default_config_file,
            config_dir,
        }
    }

    /// Test constructor with explicit inputs.
    pub fn with_sources(
        env: Vec<(String, String)>,
        cli_config_override: Option<PathBuf>,
        default_config_file: Option<PathBuf>,
        config_dir: Option<PathBuf>,
    ) -> Self {
        CredentialLookup {
            env,
            cli_config_override,
            default_config_file,
            config_dir,
        }
    }

    /// Resolve a token for `host` honoring flag > env > file (R2.1). On no
    /// match, exit-3 error with the `tfrm login` hint; mentions any
    /// configured `credentials_helper` (R2.1a).
    pub fn resolve(&self, host: &str, flag_token: Option<&str>) -> Result<Credential> {
        if let Some(token) = flag_token {
            return Ok(Credential {
                token: token.to_string(),
                source: CredentialSource::Flag,
            });
        }

        let host_norm = normalize_host(host)
            .ok_or_else(|| Error::Usage(format!("invalid hostname: {host}")))?;

        if let Some(cred) = self.env_credential(&host_norm) {
            return Ok(cred);
        }

        let files = self.load_file_credentials();
        if let Some((token, path)) = files.tokens.get(&host_norm) {
            return Ok(Credential {
                token: token.clone(),
                source: CredentialSource::File(path.clone()),
            });
        }

        let mut msg = format!("no credentials found for {host}");
        if let Some(helper) = &files.helper {
            msg.push_str(&format!(
                "; a credentials_helper \"{helper}\" is configured, but tfrm does not \
                 execute credential helpers in v0.1"
            ));
        }
        msg.push_str(&format!(
            ". Run `tfrm login {host}`, or set TF_TOKEN_{}",
            host_norm.replace('-', "__").replace('.', "_")
        ));
        Err(Error::Auth(msg))
    }

    fn env_credential(&self, host_norm: &str) -> Option<Credential> {
        // Sort for determinism when several names map to the same host.
        let mut vars: Vec<&(String, String)> = self
            .env
            .iter()
            .filter(|(name, _)| name.starts_with("TF_TOKEN_"))
            .collect();
        vars.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, value) in vars {
            let raw = &name["TF_TOKEN_".len()..];
            if normalize_host(&unmangle_env_host(raw)).as_deref() == Some(host_norm) {
                return Some(Credential {
                    token: value.clone(),
                    source: CredentialSource::Env(name.clone()),
                });
            }
        }
        None
    }

    /// Merge file-based credentials the way terraform does: the main config
    /// file first, then (unless `TF_CLI_CONFIG_FILE` overrides) every
    /// `*.tfrc` / `*.tfrc.json` in the config dir in name order, later files
    /// overriding earlier ones per host.
    fn load_file_credentials(&self) -> FileCredentials {
        let mut acc = FileCredentials::default();

        let main = self
            .cli_config_override
            .as_ref()
            .or(self.default_config_file.as_ref());
        if let Some(path) = main {
            if path.is_file() {
                acc.merge_file(path);
            }
        }

        if self.cli_config_override.is_none() {
            if let Some(dir) = &self.config_dir {
                let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| {
                        p.is_file()
                            && p.file_name().is_some_and(|n| {
                                let n = n.to_string_lossy();
                                n.ends_with(".tfrc") || n.ends_with(".tfrc.json")
                            })
                    })
                    .collect();
                entries.sort();
                for path in entries {
                    acc.merge_file(&path);
                }
            }
        }

        acc
    }
}

#[derive(Debug, Default)]
struct FileCredentials {
    /// normalized host → (token, file it came from); later merges override.
    tokens: BTreeMap<String, (String, PathBuf)>,
    /// Name of a configured credentials_helper, if any (R2.1a).
    helper: Option<String>,
}

impl FileCredentials {
    fn merge_file(&mut self, path: &Path) {
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        let is_json = path.to_string_lossy().ends_with(".json");
        if is_json {
            self.merge_json(&text, path);
        } else {
            self.merge_hcl(&text, path);
        }
    }

    fn merge_hcl(&mut self, text: &str, path: &Path) {
        let Ok(body) = hcl::from_str::<hcl::Body>(text) else {
            return;
        };
        for block in body.blocks() {
            match block.identifier() {
                "credentials" => {
                    let Some(label) = block.labels().first() else {
                        continue;
                    };
                    let Some(host) = normalize_host(label.as_str()) else {
                        continue;
                    };
                    for attr in block.body().attributes() {
                        if attr.key() == "token" {
                            if let hcl::Expression::String(token) = attr.expr() {
                                self.tokens
                                    .insert(host.clone(), (token.clone(), path.to_path_buf()));
                            }
                        }
                    }
                }
                "credentials_helper" => {
                    if let Some(label) = block.labels().first() {
                        self.helper = Some(label.as_str().to_string());
                    }
                }
                _ => {}
            }
        }
    }

    fn merge_json(&mut self, text: &str, path: &Path) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
            return;
        };
        if let Some(creds) = value.get("credentials").and_then(|c| c.as_object()) {
            for (host, entry) in creds {
                let Some(host) = normalize_host(host) else {
                    continue;
                };
                if let Some(token) = entry.get("token").and_then(|t| t.as_str()) {
                    self.tokens
                        .insert(host, (token.to_string(), path.to_path_buf()));
                }
            }
        }
        if let Some(helpers) = value.get("credentials_helper").and_then(|h| h.as_object()) {
            if let Some(name) = helpers.keys().next() {
                self.helper = Some(name.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_lookup(vars: &[(&str, &str)]) -> CredentialLookup {
        CredentialLookup::with_sources(
            vars.iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            None,
            None,
            None,
        )
    }

    fn resolve_env(vars: &[(&str, &str)], host: &str) -> Result<Credential> {
        env_lookup(vars).resolve(host, None)
    }

    // Mangling cases ported from terraform's credentials_test.go.

    #[test]
    fn env_var_with_underscores_for_dots() {
        let cred = resolve_env(
            &[("TF_TOKEN_configured_example_com", "configured-by-env")],
            "configured.example.com",
        )
        .unwrap();
        assert_eq!(cred.token, "configured-by-env");
        assert_eq!(
            cred.source,
            CredentialSource::Env("TF_TOKEN_configured_example_com".into())
        );
    }

    #[test]
    fn punycode_name_set_in_environment() {
        let cred = resolve_env(
            &[("TF_TOKEN_env_xn--eckwd4c7cu47r2wf_com", "configured-by-env")],
            "env.ドメイン名例.com",
        )
        .unwrap();
        assert_eq!(cred.token, "configured-by-env");
    }

    #[test]
    fn hyphens_can_be_encoded_as_double_underscores() {
        let cred = resolve_env(
            &[("TF_TOKEN_env_xn____caf__dma_fr", "configured-by-fallback")],
            "env.café.fr",
        )
        .unwrap();
        assert_eq!(cred.token, "configured-by-fallback");
    }

    #[test]
    fn periods_are_ok() {
        let cred = resolve_env(
            &[("TF_TOKEN_configured.example.com", "configured-by-env")],
            "configured.example.com",
        )
        .unwrap();
        assert_eq!(cred.token, "configured-by-env");
    }

    #[test]
    fn casing_is_insensitive() {
        let cred = resolve_env(
            &[(
                "TF_TOKEN_CONFIGUREDUPPERCASE_EXAMPLE_COM",
                "configured-by-env",
            )],
            "configureduppercase.example.com",
        )
        .unwrap();
        assert_eq!(cred.token, "configured-by-env");
    }

    #[test]
    fn unrelated_env_vars_do_not_match() {
        let err = resolve_env(
            &[("TF_TOKEN_other_example_com", "nope")],
            "configured.example.com",
        )
        .unwrap_err();
        assert_eq!(err.exit_code(), 3);
        assert!(err.to_string().contains("tfrm login"), "{err}");
    }

    // File sources and precedence, driven by testdata/cliconfig fixtures.

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/cliconfig")
            .join(name)
    }

    #[test]
    fn terraformrc_credentials_block_resolves() {
        let lookup =
            CredentialLookup::with_sources(vec![], None, Some(fixture("terraformrc")), None);
        let cred = lookup.resolve("app.terraform.io", None).unwrap();
        assert_eq!(cred.token, "token-from-terraformrc");
        assert!(matches!(cred.source, CredentialSource::File(_)));
    }

    #[test]
    fn credentials_tfrc_json_resolves_via_config_dir() {
        let lookup =
            CredentialLookup::with_sources(vec![], None, None, Some(fixture("terraform.d")));
        let cred = lookup.resolve("app.terraform.io", None).unwrap();
        assert_eq!(cred.token, "token-from-credentials-json");
    }

    #[test]
    fn config_dir_json_overrides_terraformrc_for_same_host() {
        // terraform merges the config dir after the main file; later wins.
        let lookup = CredentialLookup::with_sources(
            vec![],
            None,
            Some(fixture("terraformrc")),
            Some(fixture("terraform.d")),
        );
        let cred = lookup.resolve("app.terraform.io", None).unwrap();
        assert_eq!(cred.token, "token-from-credentials-json");
    }

    #[test]
    fn tf_cli_config_file_override_skips_config_dir() {
        // Terraform interprets TF_CLI_CONFIG_FILE as "ignore default files":
        // the config dir (and its credentials.tfrc.json) is not read.
        let lookup = CredentialLookup::with_sources(
            vec![],
            Some(fixture("terraformrc")),
            None,
            Some(fixture("terraform.d")),
        );
        let cred = lookup.resolve("app.terraform.io", None).unwrap();
        assert_eq!(cred.token, "token-from-terraformrc");
    }

    #[test]
    fn env_beats_file() {
        let lookup = CredentialLookup::with_sources(
            vec![("TF_TOKEN_app_terraform_io".into(), "token-from-env".into())],
            None,
            Some(fixture("terraformrc")),
            None,
        );
        let cred = lookup.resolve("app.terraform.io", None).unwrap();
        assert_eq!(cred.token, "token-from-env");
    }

    #[test]
    fn flag_beats_env_and_file() {
        let lookup = CredentialLookup::with_sources(
            vec![("TF_TOKEN_app_terraform_io".into(), "token-from-env".into())],
            None,
            Some(fixture("terraformrc")),
            None,
        );
        let cred = lookup
            .resolve("app.terraform.io", Some("token-from-flag"))
            .unwrap();
        assert_eq!(cred.token, "token-from-flag");
        assert_eq!(cred.source, CredentialSource::Flag);
    }

    #[test]
    fn helper_only_config_names_the_helper_in_the_error() {
        let lookup = CredentialLookup::with_sources(
            vec![],
            None,
            Some(fixture("terraformrc-helper-only")),
            None,
        );
        let err = lookup.resolve("app.terraform.io", None).unwrap_err();
        assert_eq!(err.exit_code(), 3);
        let msg = err.to_string();
        assert!(msg.contains("credentials_helper \"osenv\""), "{msg}");
        assert!(msg.contains("does not execute credential helpers"), "{msg}");
        assert!(msg.contains("tfrm login"), "{msg}");
    }

    #[test]
    fn no_sources_at_all_gives_login_hint_and_env_name() {
        let err = env_lookup(&[])
            .resolve("app.terraform.io", None)
            .unwrap_err();
        assert_eq!(err.exit_code(), 3);
        let msg = err.to_string();
        assert!(msg.contains("run `tfrm login app.terraform.io`") || msg.contains("tfrm login"));
        assert!(msg.contains("TF_TOKEN_app_terraform_io"), "{msg}");
    }
}
