//! `tfrm runs …` commands.

use serde_json::Value;
use tfrm_core::{runs, workspaces, Error, Result};

use crate::app::App;
use crate::table;

/// Resolve the effective workspace name to its API id.
pub async fn workspace_id(app: &App, client: &tfrm_core::client::Client) -> Result<String> {
    let (name, _) = app.ctx.resolve_workspace(app.global.workspace.as_deref())?;
    let org = app.org()?;
    let ws = workspaces::get_by_name(client, &org, &name).await?;
    ws.get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| Error::Other(format!("workspace {name}: response missing id")))
}

pub async fn list(app: &App, limit: usize, status: Option<&str>) -> Result<()> {
    let client = app.client()?;
    let ws_id = workspace_id(app, &client).await?;
    let rows = runs::list(&client, &ws_id, status, limit).await?;

    if app.json_output() {
        println!(
            "{}",
            serde_json::to_string_pretty(&rows)
                .map_err(|e| Error::Other(format!("cannot serialize runs: {e}")))?
        );
        return Ok(());
    }

    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|run| {
            vec![
                // R4.2: distinct marker for confirmable runs.
                if run.confirmable {
                    ">".into()
                } else {
                    "".into()
                },
                run.id.clone(),
                run.status.clone(),
                run.created_at.clone().unwrap_or_default(),
                run.commit_sha
                    .as_deref()
                    .map(|sha| sha.chars().take(8).collect())
                    .unwrap_or_default(),
                run.source.clone().unwrap_or_default(),
                run.message.clone().unwrap_or_default(),
            ]
        })
        .collect();
    print!(
        "{}",
        table::render(
            &["", "RUN ID", "STATUS", "CREATED", "COMMIT", "SOURCE", "MESSAGE"],
            &table_rows
        )
    );
    if rows.iter().any(|r| r.confirmable) {
        println!("\n> = awaiting confirmation (tfrm runs apply <RUN_ID>)");
    }
    Ok(())
}
