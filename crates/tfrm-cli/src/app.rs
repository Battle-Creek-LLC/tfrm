//! Shared command context: discovered project config plus an
//! authenticated API client built from the global flags.

use tfrm_core::client::Client;
use tfrm_core::config::Context;
use tfrm_core::credentials::CredentialLookup;
use tfrm_core::{Error, Result};

use crate::cli::GlobalArgs;

pub struct App {
    pub ctx: Context,
    pub global: GlobalArgs,
}

impl App {
    pub fn new(global: GlobalArgs) -> Result<Self> {
        let cwd = std::env::current_dir()
            .map_err(|e| Error::Other(format!("cannot determine working directory: {e}")))?;
        let ctx = Context::discover(&cwd)?;
        Ok(App { ctx, global })
    }

    /// Resolve credentials for the configured host and build a client.
    pub fn client(&self) -> Result<Client> {
        let host = self.ctx.hostname();
        let cred = CredentialLookup::from_os().resolve(&host, self.global.token.as_deref())?;
        Client::new(&host, cred.token)
    }

    pub fn org(&self) -> Result<String> {
        self.ctx.resolve_org(self.global.org.as_deref())
    }

    /// The workspace name currently in effect, if any (used for the
    /// selected-marker; errors are not fatal here).
    pub fn selected_workspace(&self) -> Option<String> {
        self.ctx
            .resolve_workspace(self.global.workspace.as_deref())
            .ok()
            .map(|(ws, _)| ws)
    }

    pub fn json_output(&self) -> bool {
        matches!(self.global.format, crate::cli::Format::Json)
    }
}
