//! Plan-pair diff (R6.1–R6.7), keyed by resource address.
//!
//! Like `show`, this module is a redaction boundary: sensitive values are
//! compared in-process on the decoded JSON and never stored in the
//! report. A sensitive attribute is either omitted (underlying values
//! equal) or listed as `(sensitive — differs)` with no values (R6.4).

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

use crate::client::Client;
use crate::error::{Error, Result};

/// Marker for a sensitive attribute whose values differ (R6.4).
pub const SENSITIVE_DIFFERS: &str = "(sensitive — differs)";
/// Marker for values only known after apply, reused for comparison.
pub const UNKNOWN: &str = "(known after apply)";

#[derive(Debug, Serialize)]
pub struct DiffReport {
    pub run_a: String,
    pub run_b: String,
    /// Resources changed only in A (address, action).
    pub only_in_a: Vec<ResourceRef>,
    /// Resources changed only in B.
    pub only_in_b: Vec<ResourceRef>,
    /// Resources changed in both, with differing changes.
    pub differing: Vec<ResourceDiff>,
    /// Count of resources changed identically in both.
    pub identical_count: usize,
    /// Identical addresses, populated only with `--all` (R6.2).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identical: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct ResourceRef {
    pub address: String,
    pub action: String,
}

#[derive(Debug, Serialize)]
pub struct ResourceDiff {
    pub address: String,
    pub action_a: String,
    pub action_b: String,
    pub attributes: Vec<AttributeDiff>,
}

/// One attribute with differing after-values. For sensitive attributes
/// `sensitive_differs` is true and both values are null — the values
/// never enter the report (R6.4).
#[derive(Debug, Serialize)]
pub struct AttributeDiff {
    pub name: String,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub a: Value,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub b: Value,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub sensitive_differs: bool,
}

impl DiffReport {
    pub fn has_differences(&self) -> bool {
        !self.only_in_a.is_empty() || !self.only_in_b.is_empty() || !self.differing.is_empty()
    }
}

/// Resolve the workspace's most recent applied run for `--against
/// latest-applied` (R6.1): the runs listing filtered to applied, newest
/// first, first page, one item. Deliberately not `latest-run` (deprecated)
/// or `current-run` (any status).
pub async fn latest_applied_run(client: &Client, workspace_id: &str) -> Result<String> {
    let doc = client
        .get_json(&format!(
            "/api/v2/workspaces/{workspace_id}/runs?filter%5Bstatus%5D=applied&page%5Bsize%5D=1"
        ))
        .await?;
    doc.pointer("/data/0/id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            Error::NotFound(format!(
                "workspace {workspace_id} has no applied run to diff against"
            ))
        })
}

fn truthy_mask(mask: Option<&Value>) -> bool {
    match mask {
        Some(Value::Bool(b)) => *b,
        Some(Value::Object(m)) => !m.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        _ => false,
    }
}

fn classify_actions(change: &Value) -> String {
    let actions: Vec<&str> = change
        .get("actions")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    match actions.as_slice() {
        ["create"] => "create",
        ["update"] => "update",
        ["delete"] => "delete",
        ["read"] => "read",
        ["delete", "create"] | ["create", "delete"] => "replace",
        _ => "no-op",
    }
    .to_string()
}

/// Non-no-op changes of a plan, keyed by address.
fn changes_by_address(plan: &Value) -> BTreeMap<String, &Value> {
    plan.get("resource_changes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|rc| {
            let change = rc.get("change")?;
            if classify_actions(change) == "no-op" {
                return None;
            }
            let address = rc.get("address")?.as_str()?;
            Some((address.to_string(), change))
        })
        .collect()
}

/// The comparable after-value of one attribute: the raw value, or the
/// unknown marker when `after_unknown` is truthy (two unknowns compare
/// equal — nothing more can be said before apply).
fn comparable_after<'v>(change: &'v Value, key: &str) -> Option<Value> {
    let unknown = truthy_mask(change.get("after_unknown").and_then(|m| m.get(key)));
    if unknown {
        return Some(Value::String(UNKNOWN.into()));
    }
    let after: Option<&'v Value> = change.get("after").and_then(|m| m.get(key));
    after.cloned()
}

fn attr_sensitive(change: &Value, key: &str) -> bool {
    truthy_mask(change.get("after_sensitive").and_then(|m| m.get(key)))
        || truthy_mask(change.get("before_sensitive").and_then(|m| m.get(key)))
}

/// Diff two plans. Sensitive equality is computed here, inside the
/// boundary; sensitive values never reach the returned report.
pub fn diff_plans(
    run_a: &str,
    run_b: &str,
    plan_a: &Value,
    plan_b: &Value,
    include_identical: bool,
) -> DiffReport {
    let a_changes = changes_by_address(plan_a);
    let b_changes = changes_by_address(plan_b);

    let mut only_in_a = Vec::new();
    let mut only_in_b = Vec::new();
    let mut differing = Vec::new();
    let mut identical = Vec::new();

    for (address, change_a) in &a_changes {
        match b_changes.get(address) {
            None => only_in_a.push(ResourceRef {
                address: address.clone(),
                action: classify_actions(change_a),
            }),
            Some(change_b) => {
                let action_a = classify_actions(change_a);
                let action_b = classify_actions(change_b);
                let attributes = diff_attributes(change_a, change_b);
                if action_a == action_b && attributes.is_empty() {
                    identical.push(address.clone());
                } else {
                    differing.push(ResourceDiff {
                        address: address.clone(),
                        action_a,
                        action_b,
                        attributes,
                    });
                }
            }
        }
    }
    for (address, change_b) in &b_changes {
        if !a_changes.contains_key(address) {
            only_in_b.push(ResourceRef {
                address: address.clone(),
                action: classify_actions(change_b),
            });
        }
    }

    DiffReport {
        run_a: run_a.to_string(),
        run_b: run_b.to_string(),
        only_in_a,
        only_in_b,
        differing,
        identical_count: identical.len(),
        identical: include_identical.then_some(identical),
    }
}

fn diff_attributes(change_a: &Value, change_b: &Value) -> Vec<AttributeDiff> {
    let mut keys: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for change in [change_a, change_b] {
        for map_name in ["after", "after_unknown"] {
            if let Some(map) = change.get(map_name).and_then(Value::as_object) {
                keys.extend(map.keys().cloned());
            }
        }
    }

    let mut diffs = Vec::new();
    for key in keys {
        let value_a = comparable_after(change_a, &key);
        let value_b = comparable_after(change_b, &key);
        if value_a == value_b {
            continue; // identical on both sides — sensitive or not, omit
        }
        let sensitive = attr_sensitive(change_a, &key) || attr_sensitive(change_b, &key);
        if sensitive {
            // R6.4: values differ but must not be shown.
            diffs.push(AttributeDiff {
                name: key,
                a: Value::Null,
                b: Value::Null,
                sensitive_differs: true,
            });
        } else {
            diffs.push(AttributeDiff {
                name: key,
                a: value_a.unwrap_or(Value::Null),
                b: value_b.unwrap_or(Value::Null),
                sensitive_differs: false,
            });
        }
    }
    diffs
}

/// Render the diff report as text; values are pre-redacted.
pub fn render_text(report: &DiffReport) -> String {
    let mut out = String::new();
    out.push_str(&format!("Diff {} -> {}\n", report.run_a, report.run_b));

    if !report.has_differences() {
        out.push_str("\nNo differences.\n");
        if report.identical_count > 0 {
            out.push_str(&format!(
                "{} resource change(s) identical in both plans.\n",
                report.identical_count
            ));
        }
        return out;
    }

    if !report.only_in_a.is_empty() {
        out.push_str(&format!("\nOnly in {}:\n", report.run_a));
        for r in &report.only_in_a {
            out.push_str(&format!("  {} ({})\n", r.address, r.action));
        }
    }
    if !report.only_in_b.is_empty() {
        out.push_str(&format!("\nOnly in {}:\n", report.run_b));
        for r in &report.only_in_b {
            out.push_str(&format!("  {} ({})\n", r.address, r.action));
        }
    }
    if !report.differing.is_empty() {
        out.push_str("\nDiffering changes:\n");
        for r in &report.differing {
            out.push_str(&format!("  ~ {}", r.address));
            if r.action_a != r.action_b {
                out.push_str(&format!("  (action: {} -> {})", r.action_a, r.action_b));
            }
            out.push('\n');
            for attr in &r.attributes {
                if attr.sensitive_differs {
                    out.push_str(&format!("      {}: {SENSITIVE_DIFFERS}\n", attr.name));
                } else {
                    out.push_str(&format!(
                        "      {}: {} | {}\n",
                        attr.name,
                        compact(&attr.a),
                        compact(&attr.b)
                    ));
                }
            }
        }
    }
    match &report.identical {
        Some(addresses) if !addresses.is_empty() => {
            out.push_str("\nIdentical in both:\n");
            for address in addresses {
                out.push_str(&format!("  {address}\n"));
            }
        }
        _ if report.identical_count > 0 => {
            out.push_str(&format!(
                "\nIdentical in both: {} resource(s) (use --all to list)\n",
                report.identical_count
            ));
        }
        _ => {}
    }
    out
}

fn compact(value: &Value) -> String {
    match value {
        Value::String(s) if s == UNKNOWN => s.clone(),
        other => serde_json::to_string(other).unwrap_or_else(|_| "null".into()),
    }
}
