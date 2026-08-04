//! Plan JSON fetch (R5.6, fetch half).
//!
//! `GET /runs/:id/plan/json-output` answers 307 with a pre-signed URL
//! valid for about a minute: follow it immediately and send **no**
//! Authorization header (the URL itself is the credential, and leaking a
//! bearer token to the archive host would violate R8.3). A 403 means the
//! token has write but not admin (A2): fall back to the plan record's
//! summary counts plus raw log text.

use serde::Serialize;
use serde_json::Value;

use crate::client::Client;
use crate::error::{Error, Result};

/// Outcome of a plan-JSON fetch.
#[derive(Debug)]
pub enum PlanFetch {
    /// Full machine-readable plan JSON (admin token).
    Full(Value),
    /// 403 fallback: what the plan record alone can tell us.
    Summary(PlanRecordSummary),
}

/// Change counts from the plan record (`resource-additions` etc.), used
/// when json-output is forbidden (R5.6).
#[derive(Debug, Clone, Serialize)]
pub struct PlanRecordSummary {
    pub additions: i64,
    pub changes: i64,
    pub destructions: i64,
    /// Raw plan log text, when the log could be fetched.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log: Option<String>,
}

/// Fetch a run's plan JSON, or the summary fallback on 403.
pub async fn fetch(client: &Client, run_id: &str) -> Result<PlanFetch> {
    let resp = client
        .get_raw(&format!("/api/v2/runs/{run_id}/plan/json-output"))
        .await?;

    match resp.status().as_u16() {
        307 => {
            let location = resp
                .headers()
                .get("Location")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| {
                    Error::Other("plan json-output 307 carried no Location header".into())
                })?
                .to_string();
            let value = fetch_presigned(&location).await?;
            Ok(PlanFetch::Full(value))
        }
        // Some TFE builds answer 200 directly.
        200 => {
            let value = resp
                .json()
                .await
                .map_err(|e| Error::Other(format!("invalid plan JSON: {e}")))?;
            Ok(PlanFetch::Full(value))
        }
        403 => Ok(PlanFetch::Summary(
            fetch_record_summary(client, run_id).await?,
        )),
        404 => Err(Error::NotFound(format!(
            "run {run_id} has no plan JSON (run or plan not found)"
        ))),
        _ => {
            Client::check_status(resp).await?;
            Err(Error::Other(
                "unexpected response fetching plan JSON".into(),
            ))
        }
    }
}

/// GET the pre-signed URL with a bare client: no Authorization header,
/// single immediate request (the URL already embeds its credential).
async fn fetch_presigned(url: &str) -> Result<Value> {
    let bare = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| Error::Other(format!("failed to build HTTP client: {e}")))?;
    let resp = bare
        .get(url)
        .send()
        .await
        .map_err(|e| Error::Other(format!("pre-signed plan fetch failed: {e}")))?;
    let resp = Client::check_status(resp).await?;
    resp.json()
        .await
        .map_err(|e| Error::Other(format!("invalid plan JSON from pre-signed URL: {e}")))
}

/// Build the degraded summary from the plan record; the log text is
/// best-effort (`log-read-url` is pre-signed too).
async fn fetch_record_summary(client: &Client, run_id: &str) -> Result<PlanRecordSummary> {
    let doc = client
        .get_json(&format!("/api/v2/runs/{run_id}/plan"))
        .await?;
    let attr = |name: &str| -> i64 {
        doc.pointer(&format!("/data/attributes/{name}"))
            .and_then(Value::as_i64)
            .unwrap_or(0)
    };
    let log = match doc
        .pointer("/data/attributes/log-read-url")
        .and_then(Value::as_str)
    {
        Some(url) => fetch_log_text(url).await,
        None => None,
    };
    Ok(PlanRecordSummary {
        additions: attr("resource-additions"),
        changes: attr("resource-changes"),
        destructions: attr("resource-destructions"),
        log,
    })
}

async fn fetch_log_text(url: &str) -> Option<String> {
    let resp = reqwest::get(url).await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.text().await.ok()
}
