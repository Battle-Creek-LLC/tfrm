//! Workspaces API (R3.1–R3.3): list with current-run status, and
//! existence checks backing `workspace select`.

use serde::Serialize;
use serde_json::Value;

use crate::client::Client;
use crate::error::{Error, Result};

/// One row of `workspace list` — also the documented `--format json`
/// shape (R8.1).
#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceRow {
    pub name: String,
    /// Status of the workspace's current run, when one exists.
    pub current_run_status: Option<String>,
    /// VCS repo identifier (e.g. `org/repo`) when VCS-connected.
    pub vcs_repo: Option<String>,
    pub latest_change_at: Option<String>,
    /// True for the currently selected workspace.
    pub selected: bool,
}

/// List the org's workspaces, newest data first as the API returns them,
/// resolving each workspace's current-run status via `include=current_run`
/// (one request per page, no N+1). Default page size 20 (R3.1).
pub async fn list(client: &Client, org: &str, selected: Option<&str>) -> Result<Vec<WorkspaceRow>> {
    let (data, included) = client
        .get_paginated(
            &format!("/api/v2/organizations/{org}/workspaces"),
            &[("include", "current_run")],
            20,
            None,
        )
        .await?;

    let run_status = |run_id: &str| -> Option<String> {
        included
            .iter()
            .find(|inc| {
                inc.get("type").and_then(Value::as_str) == Some("runs")
                    && inc.get("id").and_then(Value::as_str) == Some(run_id)
            })
            .and_then(|run| run.pointer("/attributes/status"))
            .and_then(Value::as_str)
            .map(str::to_string)
    };

    Ok(data
        .iter()
        .map(|ws| {
            let name = ws
                .pointer("/attributes/name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let current_run_status = ws
                .pointer("/relationships/current-run/data/id")
                .and_then(Value::as_str)
                .and_then(run_status);
            WorkspaceRow {
                selected: selected == Some(name.as_str()),
                name,
                current_run_status,
                vcs_repo: ws
                    .pointer("/attributes/vcs-repo/identifier")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                latest_change_at: ws
                    .pointer("/attributes/latest-change-at")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            }
        })
        .collect())
}

/// Fetch one workspace by name, mapping the API's 404 onto the R3.2
/// exit-4 shape. Returns the raw workspace document (`data`).
pub async fn get_by_name(client: &Client, org: &str, name: &str) -> Result<Value> {
    match client
        .get_json(&format!("/api/v2/organizations/{org}/workspaces/{name}"))
        .await
    {
        Ok(doc) => Ok(doc.get("data").cloned().unwrap_or(Value::Null)),
        Err(Error::Api { status: 404, .. }) => Err(Error::NotFound(format!(
            "workspace {name} not found in organization {org}"
        ))),
        Err(e) => Err(e),
    }
}
