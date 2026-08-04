//! Run actions (R7.1–R7.9): apply/discard/cancel POSTs and policy-check
//! handling. The client maps 409 → exit 6 (R7.9); this module adds the
//! action-specific 403 messages (R7.5) and the policy override protocol
//! (R7.2).

use serde_json::{json, Value};

use crate::client::Client;
use crate::error::{Error, Result};

/// One policy check attached to a run.
#[derive(Debug, Clone)]
pub struct PolicyCheck {
    pub id: String,
    /// `passed` | `soft_failed` | `hard_failed` | …
    pub status: String,
    /// The `can-override` permission on the check (R7.2).
    pub can_override: bool,
}

pub async fn policy_checks(client: &Client, run_id: &str) -> Result<Vec<PolicyCheck>> {
    let doc = client
        .get_json(&format!("/api/v2/runs/{run_id}/policy-checks"))
        .await?;
    Ok(doc
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|check| {
            Some(PolicyCheck {
                id: check.get("id")?.as_str()?.to_string(),
                status: check.pointer("/attributes/status")?.as_str()?.to_string(),
                can_override: check
                    .pointer("/attributes/permissions/can-override")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        })
        .collect())
}

/// POST the soft-mandatory override. The endpoint takes no body (R7.2).
pub async fn override_policy(client: &Client, policy_check_id: &str) -> Result<()> {
    match client
        .post(
            &format!("/api/v2/policy-checks/{policy_check_id}/actions/override"),
            None,
        )
        .await
    {
        Ok(_) => Ok(()),
        Err(Error::Api { status: 403, .. }) => Err(Error::Auth(format!(
            "the token lacks the can-override permission on policy check {policy_check_id}"
        ))),
        Err(e) => Err(e),
    }
}

/// POST one of the run actions (`apply`, `discard`, `cancel`,
/// `force-cancel`) with the optional `comment` body field (R7.8). Success
/// is 202 Accepted — accepted, not finished.
pub async fn run_action(
    client: &Client,
    run_id: &str,
    action: &str,
    comment: Option<&str>,
) -> Result<()> {
    let body = comment.map(|c| json!({ "comment": c }));
    match client
        .post(&format!("/api/v2/runs/{run_id}/actions/{action}"), body)
        .await
    {
        Ok(_) => Ok(()),
        Err(Error::Api { status: 403, .. }) => Err(Error::Auth(format!(
            "the token lacks \"write\" permission on the workspace (cannot {action} run {run_id})"
        ))),
        Err(e) => Err(e),
    }
}
