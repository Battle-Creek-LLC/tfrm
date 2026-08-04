//! `tfrm workspace list|select|current` (R3.1–R3.3).

use tfrm_core::workspaces;
use tfrm_core::{Error, Result};

use crate::app::App;
use crate::table;

pub async fn list(app: &App) -> Result<()> {
    let client = app.client()?;
    let org = app.org()?;
    let selected = app.selected_workspace();
    let rows = workspaces::list(&client, &org, selected.as_deref()).await?;

    if app.json_output() {
        println!(
            "{}",
            serde_json::to_string_pretty(&rows)
                .map_err(|e| Error::Other(format!("cannot serialize workspaces: {e}")))?
        );
        return Ok(());
    }

    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|ws| {
            vec![
                if ws.selected { "*".into() } else { "".into() },
                ws.name.clone(),
                ws.current_run_status.clone().unwrap_or_default(),
                ws.vcs_repo.clone().unwrap_or_default(),
                ws.latest_change_at.clone().unwrap_or_default(),
            ]
        })
        .collect();
    print!(
        "{}",
        table::render(
            &["", "NAME", "RUN STATUS", "VCS REPO", "LATEST CHANGE"],
            &table_rows
        )
    );
    Ok(())
}

pub async fn select(app: &App, name: &str) -> Result<()> {
    let client = app.client()?;
    let org = app.org()?;
    // Verify existence first (exit 4 on unknown) so a typo never persists.
    workspaces::get_by_name(&client, &org, name).await?;
    let path = app.ctx.select_workspace(name)?;
    println!(
        "selected workspace {name} (persisted to {})",
        path.display()
    );
    Ok(())
}

pub fn current(app: &App) -> Result<()> {
    let (ws, source) = app.ctx.resolve_workspace(app.global.workspace.as_deref())?;
    println!("{ws} (from {source})");
    Ok(())
}
