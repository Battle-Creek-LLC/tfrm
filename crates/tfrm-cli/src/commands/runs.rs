//! `tfrm runs …` commands.

use serde_json::Value;
use tfrm_core::plan::PlanFetch;
use tfrm_core::{plan, runs, show, workspaces, Error, Result};

use crate::app::App;
use crate::table;

/// Run statuses during which the plan is still producing output (R5.5).
const PLANNING_STATUSES: &[&str] = &[
    "pending",
    "fetching",
    "queuing",
    "plan_queued",
    "planning",
    "cost_estimating",
    "policy_checking",
];

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

pub async fn show(app: &App, run_id: &str) -> Result<()> {
    let client = app.client()?;
    let (mut meta, _run) = runs::get_meta(&client, run_id).await?;

    // R5.5: while the plan is still running, stream its log, then render.
    if PLANNING_STATUSES.contains(&meta.status.as_str()) {
        stream_plan_log(&client, run_id).await;
        let (refreshed, _) = runs::get_meta(&client, run_id).await?;
        meta = refreshed;
    }

    match plan::fetch(&client, run_id).await? {
        PlanFetch::Full(value) => {
            let report = show::build_report(meta, &value);
            if app.json_output() {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&report)
                        .map_err(|e| Error::Other(format!("cannot serialize report: {e}")))?
                );
            } else {
                print!("{}", show::render_text(&report));
            }
        }
        PlanFetch::Summary(summary) => {
            // R5.6: degrade with a warning, exit 0.
            eprintln!(
                "warning: attribute-level detail requires workspace admin on the token; \
                 showing the plan summary only"
            );
            let report = show::build_degraded(meta, &summary);
            if app.json_output() {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&report)
                        .map_err(|e| Error::Other(format!("cannot serialize report: {e}")))?
                );
                if let Some(log) = &summary.log {
                    eprintln!("{log}");
                }
            } else {
                print!("{}", show::render_text(&report));
                if let Some(log) = &summary.log {
                    println!("\nPlan log:\n{log}");
                }
            }
        }
    }
    Ok(())
}

/// Poll the run until it leaves the planning states, printing new plan-log
/// text as it appears (stderr keeps stdout machine-parseable, R8.2).
async fn stream_plan_log(client: &tfrm_core::client::Client, run_id: &str) {
    let mut printed = 0usize;
    loop {
        let log_url = client
            .get_json(&format!("/api/v2/runs/{run_id}/plan"))
            .await
            .ok()
            .and_then(|doc| {
                doc.pointer("/data/attributes/log-read-url")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            });
        if let Some(url) = log_url {
            if let Ok(resp) = reqwest::get(&url).await {
                if let Ok(text) = resp.text().await {
                    if text.len() > printed {
                        eprint!("{}", &text[printed..]);
                        printed = text.len();
                    }
                }
            }
        }
        match runs::get_meta(client, run_id).await {
            Ok((meta, _)) if PLANNING_STATUSES.contains(&meta.status.as_str()) => {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            _ => break,
        }
    }
}
