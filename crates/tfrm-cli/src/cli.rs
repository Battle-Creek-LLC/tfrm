use clap::{Args, Parser, Subcommand, ValueEnum};

/// tfrm — HCP Terraform runs from the terminal: select a workspace, list
/// runs, view and diff plans with sensitive-value redaction, and apply
/// VCS-triggered runs.
#[derive(Debug, Parser)]
#[command(name = "tfrm", version, max_term_width = 100)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    #[command(flatten)]
    pub global: GlobalArgs,
}

#[derive(Debug, Args)]
pub struct GlobalArgs {
    /// Workspace to operate on (overrides selection and config)
    #[arg(short = 'w', long, global = true, value_name = "NAME")]
    pub workspace: Option<String>,

    /// Organization name (overrides .tfrm.toml)
    #[arg(long, global = true, value_name = "NAME")]
    pub org: Option<String>,

    /// Output format
    #[arg(long, global = true, value_enum, default_value_t = Format::Table)]
    pub format: Format,

    /// Disable colored output
    #[arg(long, global = true)]
    pub no_color: bool,

    /// API token (overrides TF_TOKEN_* env and credential files)
    #[arg(long, global = true, value_name = "TOKEN")]
    pub token: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    Table,
    Json,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Log in to HOST via browser OAuth (PKCE) with paste fallback; stores the token
    Login {
        /// Host to authenticate against
        #[arg(value_name = "HOST", default_value = "app.terraform.io")]
        host: String,
    },

    /// Remove the stored token for HOST
    Logout {
        /// Host to remove the stored token for
        #[arg(value_name = "HOST", default_value = "app.terraform.io")]
        host: String,
    },

    /// List, select, and show workspaces
    #[command(subcommand)]
    Workspace(WorkspaceCommand),

    /// List, inspect, diff, and act on runs
    #[command(subcommand)]
    Runs(RunsCommand),

    /// Report which credential source resolves for HOST (debugging aid)
    #[command(hide = true, name = "auth-debug")]
    AuthDebug {
        #[arg(value_name = "HOST", default_value = "app.terraform.io")]
        host: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum WorkspaceCommand {
    /// List the org's workspaces
    List,

    /// Set the current workspace (persisted)
    Select {
        #[arg(value_name = "NAME")]
        name: String,
    },

    /// Show the selection and its source
    Current,
}

#[derive(Debug, Subcommand)]
pub enum RunsCommand {
    /// List recent runs
    List {
        /// Maximum number of runs to list
        #[arg(long, value_name = "N", default_value_t = 20)]
        limit: usize,

        /// Only runs with this status (maps to the API's filter[status])
        #[arg(long, value_name = "STATUS")]
        status: Option<String>,
    },

    /// Render one run's plan
    Show {
        #[arg(value_name = "RUN_ID")]
        run_id: String,
    },

    /// Diff two plans
    Diff {
        #[arg(value_name = "A")]
        a: String,
        /// Second run; omit with --against latest-applied
        #[arg(value_name = "B")]
        b: Option<String>,

        /// Diff A against a resolved run instead of naming B
        #[arg(long, value_name = "REF", value_parser = ["latest-applied"])]
        against: Option<String>,

        /// Also list resources whose changes are identical in both plans
        #[arg(long)]
        all: bool,

        /// Exit 1 when differences exist (git-diff convention)
        #[arg(long)]
        exit_code: bool,

        /// Allow diffing runs from different workspaces
        #[arg(long)]
        allow_cross_workspace: bool,
    },

    /// Confirm and apply a run awaiting confirmation
    Apply {
        #[arg(value_name = "RUN_ID")]
        run_id: String,

        /// Comment recorded in the run timeline
        #[arg(short = 'm', long, value_name = "TEXT")]
        comment: Option<String>,

        /// Skip the type-the-workspace-name confirmation prompt
        #[arg(long)]
        auto_approve: bool,

        /// Override a failed soft-mandatory policy check before applying
        #[arg(long)]
        override_policy: bool,
    },

    /// Reject a run awaiting confirmation
    Discard {
        #[arg(value_name = "RUN_ID")]
        run_id: String,

        /// Comment recorded in the run timeline
        #[arg(short = 'm', long, value_name = "TEXT")]
        comment: Option<String>,
    },

    /// Stop a run that is actively planning or applying
    Cancel {
        #[arg(value_name = "RUN_ID")]
        run_id: String,

        /// Comment recorded in the run timeline
        #[arg(short = 'm', long, value_name = "TEXT")]
        comment: Option<String>,

        /// Force-cancel (allowed only once the API reports is-force-cancelable)
        #[arg(long)]
        force: bool,
    },
}
