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
    match command {
        Command::AuthDebug { host } => {
            let lookup = tfrm_core::credentials::CredentialLookup::from_os();
            let cred = lookup.resolve(&host, app.global.token.as_deref())?;
            // Provenance only — the token itself must never be printed (R2.3).
            println!("token for {host}: {}", cred.source);
            Ok(())
        }
        Command::Login { host } => commands::login::login(&host).await,
        Command::Logout { host } => commands::login::logout(&host),
        Command::Workspace(cmd) => match cmd {
            WorkspaceCommand::List => commands::workspace::list(&app).await,
            WorkspaceCommand::Select { name } => commands::workspace::select(&app, &name).await,
            WorkspaceCommand::Current => commands::workspace::current(&app),
        },
        Command::Runs(cmd) => match cmd {
            RunsCommand::List { limit, status } => {
                commands::runs::list(&app, limit, status.as_deref()).await
            }
            RunsCommand::Show { run_id } => commands::runs::show(&app, &run_id).await,
            RunsCommand::Diff {
                a,
                b,
                against,
                all,
                exit_code,
                allow_cross_workspace,
            } => {
                commands::runs::diff(
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
            RunsCommand::Apply {
                run_id,
                comment,
                auto_approve,
                override_policy,
            } => {
                commands::runs::apply(
                    &app,
                    &run_id,
                    comment.as_deref(),
                    auto_approve,
                    override_policy,
                )
                .await
            }
            RunsCommand::Discard { run_id, comment } => {
                commands::runs::discard(&app, &run_id, comment.as_deref()).await
            }
            RunsCommand::Cancel {
                run_id,
                comment,
                force,
            } => commands::runs::cancel(&app, &run_id, comment.as_deref(), force).await,
        },
    }
}
