//! Runs API (R4.1–R4.2): list a workspace's runs with commit metadata
//! resolved in the same request.

use serde::Serialize;
use serde_json::Value;

use crate::client::Client;
use crate::error::Result;
use crate::show::RunMeta;

/// One row of `runs list` — also the documented `--format json` shape
/// (R8.1).
#[derive(Debug, Clone, Serialize)]
pub struct RunRow {
    pub id: String,
    pub status: String,
    pub created_at: Option<String>,
    /// Commit SHA of the VCS ingress that created the run, when present.
    pub commit_sha: Option<String>,
    pub message: Option<String>,
    /// The run's `source` attribute (e.g. `tfe-api`, `tfe-ui`).
    pub source: Option<String>,
    /// True when `actions.is-confirmable` (R4.2).
    pub confirmable: bool,
}

/// List a workspace's most recent runs, newest first. Commit SHAs come
/// from `include=configuration_version.ingress_attributes` in the same
/// request — one API call per page, never one per run (R4.1). `plan_only`
/// runs stay excluded because tfrm passes no `filter[plan_only]` override
/// (R4.1a — the API default).
pub async fn list(
    client: &Client,
    workspace_id: &str,
    status_filter: Option<&str>,
    limit: usize,
) -> Result<Vec<RunRow>> {
    let mut query: Vec<(&str, &str)> =
        vec![("include", "configuration_version.ingress_attributes")];
    if let Some(status) = status_filter {
        query.push(("filter[status]", status));
    }
    let (data, included) = client
        .get_paginated(
            &format!("/api/v2/workspaces/{workspace_id}/runs"),
            &query,
            20,
            Some(limit),
        )
        .await?;

    let find = |ty: &str, id: &str| -> Option<&Value> {
        included.iter().find(|inc| {
            inc.get("type").and_then(Value::as_str) == Some(ty)
                && inc.get("id").and_then(Value::as_str) == Some(id)
        })
    };

    Ok(data.iter().map(|run| row(run, &find)).collect())
}

/// Fetch one run with the includes needed for the R5.1 header, returning
/// the header metadata plus the raw run document (whose `actions` gate
/// the J3 commands).
pub async fn get_meta(client: &Client, run_id: &str) -> Result<(RunMeta, Value)> {
    let doc = client
        .get_json(&format!(
            "/api/v2/runs/{run_id}?include=workspace,configuration_version.ingress_attributes"
        ))
        .await?;
    let run = doc.get("data").cloned().unwrap_or(Value::Null);
    let empty = Vec::new();
    let included = doc
        .get("included")
        .and_then(Value::as_array)
        .unwrap_or(&empty);

    let find = |ty: &str, id: &str| -> Option<&Value> {
        included.iter().find(|inc| {
            inc.get("type").and_then(Value::as_str) == Some(ty)
                && inc.get("id").and_then(Value::as_str) == Some(id)
        })
    };

    let workspace = run
        .pointer("/relationships/workspace/data/id")
        .and_then(Value::as_str)
        .and_then(|ws_id| find("workspaces", ws_id))
        .and_then(|ws| ws.pointer("/attributes/name"))
        .and_then(Value::as_str)
        .map(str::to_string);

    let commit_sha = run
        .pointer("/relationships/configuration-version/data/id")
        .and_then(Value::as_str)
        .and_then(|cv_id| find("configuration-versions", cv_id))
        .and_then(|cv| {
            cv.pointer("/relationships/ingress-attributes/data/id")
                .and_then(Value::as_str)
        })
        .and_then(|ia_id| find("ingress-attributes", ia_id))
        .and_then(|ia| ia.pointer("/attributes/commit-sha"))
        .and_then(Value::as_str)
        .map(str::to_string);

    let attr = |ptr: &str| run.pointer(ptr).and_then(Value::as_str).map(str::to_string);
    let meta = RunMeta {
        id: run
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or(run_id)
            .to_string(),
        workspace,
        status: attr("/attributes/status").unwrap_or_default(),
        source: attr("/attributes/source"),
        commit_sha,
        message: attr("/attributes/message"),
    };
    Ok((meta, run))
}

fn row<'a>(run: &Value, find: &impl Fn(&str, &str) -> Option<&'a Value>) -> RunRow {
    let attr = |ptr: &str| run.pointer(ptr).and_then(Value::as_str).map(str::to_string);

    // run → configuration-version → ingress-attributes → commit-sha
    let commit_sha = run
        .pointer("/relationships/configuration-version/data/id")
        .and_then(Value::as_str)
        .and_then(|cv_id| find("configuration-versions", cv_id))
        .and_then(|cv| {
            cv.pointer("/relationships/ingress-attributes/data/id")
                .and_then(Value::as_str)
        })
        .and_then(|ia_id| find("ingress-attributes", ia_id))
        .and_then(|ia| {
            ia.pointer("/attributes/commit-sha")
                .and_then(Value::as_str)
                .map(str::to_string)
        });

    RunRow {
        id: run
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        status: attr("/attributes/status").unwrap_or_default(),
        created_at: attr("/attributes/created-at"),
        commit_sha,
        message: attr("/attributes/message"),
        source: attr("/attributes/source"),
        confirmable: run
            .pointer("/attributes/actions/is-confirmable")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}
