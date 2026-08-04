mod cli;

use clap::Parser;
use cli::{Cli, Command, RunsCommand, WorkspaceCommand};
use tfrm_core::Error;

fn main() {
    let cli = Cli::parse();
    if let Err(err) = run(cli) {
        eprintln!("tfrm: {err}");
        std::process::exit(err.exit_code());
    }
}

fn run(cli: Cli) -> Result<(), Error> {
    let name = match &cli.command {
        Command::AuthDebug { host } => {
            let lookup = tfrm_core::credentials::CredentialLookup::from_os();
            let cred = lookup.resolve(host, cli.global.token.as_deref())?;
            // Provenance only — the token itself must never be printed (R2.3).
            println!("token for {host}: {}", cred.source);
            return Ok(());
        }
        Command::Login { .. } => "login",
        Command::Logout { .. } => "logout",
        Command::Workspace(cmd) => match cmd {
            WorkspaceCommand::List => "workspace list",
            WorkspaceCommand::Select { .. } => "workspace select",
            WorkspaceCommand::Current => {
                let cwd = std::env::current_dir().map_err(|e| {
                    Error::Other(format!("cannot determine working directory: {e}"))
                })?;
                let ctx = tfrm_core::config::Context::discover(&cwd)?;
                let (ws, source) = ctx.resolve_workspace(cli.global.workspace.as_deref())?;
                println!("{ws} (from {source})");
                return Ok(());
            }
        },
        Command::Runs(cmd) => match cmd {
            RunsCommand::List => "runs list",
            RunsCommand::Show { .. } => "runs show",
            RunsCommand::Diff { .. } => "runs diff",
            RunsCommand::Apply { .. } => "runs apply",
            RunsCommand::Discard { .. } => "runs discard",
            RunsCommand::Cancel { .. } => "runs cancel",
        },
    };
    Err(Error::Other(format!("{name}: not implemented")))
}
