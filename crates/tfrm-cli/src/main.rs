mod app;
mod cli;
mod commands;
mod table;

use app::App;
use clap::Parser;
use cli::{Cli, Command, RunsCommand, WorkspaceCommand};
use tfrm_core::Error;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    if let Err(err) = run(cli).await {
        eprintln!("tfrm: {err}");
        std::process::exit(err.exit_code());
    }
}

async fn run(cli: Cli) -> Result<(), Error> {
    let command = cli.command;
    let app = App::new(cli.global)?;
    let name = match command {
        Command::AuthDebug { host } => {
            let lookup = tfrm_core::credentials::CredentialLookup::from_os();
            let cred = lookup.resolve(&host, app.global.token.as_deref())?;
            // Provenance only — the token itself must never be printed (R2.3).
            println!("token for {host}: {}", cred.source);
            return Ok(());
        }
        Command::Login { .. } => "login",
        Command::Logout { .. } => "logout",
        Command::Workspace(cmd) => match cmd {
            WorkspaceCommand::List => return commands::workspace::list(&app).await,
            WorkspaceCommand::Select { name } => {
                return commands::workspace::select(&app, &name).await
            }
            WorkspaceCommand::Current => return commands::workspace::current(&app),
        },
        Command::Runs(cmd) => match cmd {
            RunsCommand::List { limit, status } => {
                return commands::runs::list(&app, limit, status.as_deref()).await
            }
            RunsCommand::Show { run_id } => return commands::runs::show(&app, &run_id).await,
            RunsCommand::Diff {
                a,
                b,
                against,
                all,
                exit_code,
                allow_cross_workspace,
            } => {
                return commands::runs::diff(
                    &app,
                    commands::runs::DiffArgs {
                        a,
                        b,
                        against,
                        all,
                        exit_code,
                        allow_cross_workspace,
                    },
                )
                .await
            }
            RunsCommand::Apply { .. } => "runs apply",
            RunsCommand::Discard { .. } => "runs discard",
            RunsCommand::Cancel { .. } => "runs cancel",
        },
    };
    Err(Error::Other(format!("{name}: not implemented")))
}
