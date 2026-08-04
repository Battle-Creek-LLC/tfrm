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
        Command::Login { .. } => "login",
        Command::Logout { .. } => "logout",
        Command::Workspace(cmd) => match cmd {
            WorkspaceCommand::List => "workspace list",
            WorkspaceCommand::Select { .. } => "workspace select",
            WorkspaceCommand::Current => "workspace current",
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
