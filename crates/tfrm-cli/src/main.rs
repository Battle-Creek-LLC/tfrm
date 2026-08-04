mod cli;

use clap::Parser;
use cli::{Cli, Command, RunsCommand, WorkspaceCommand};

fn main() {
    let cli = Cli::parse();

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

    eprintln!("tfrm {name}: not implemented");
    std::process::exit(1);
}
