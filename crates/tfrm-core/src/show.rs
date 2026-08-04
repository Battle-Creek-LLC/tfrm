//! Plan rendering for `runs show` (R5.1–R5.4, R5.6 render half).
//!
//! This module is the redaction boundary (spec §9): `build_report`
//! decodes the plan JSON and produces a fully redacted `ShowReport`;
//! everything downstream (text rendering, `--format json`) only ever
//! sees markers where sensitive values were. No other module may read
//! `before`/`after` payloads.

use std::collections::BTreeSet;

use serde::Serialize;
use serde_json::Value;

use crate::plan::PlanRecordSummary;

/// Marker printed in place of a sensitive value (R5.3).
pub const SENSITIVE: &str = "(sensitive)";
/// Marker for values only known after apply (R5.4).
pub const UNKNOWN: &str = "(known after apply)";

/// Run metadata rendered in the header (R5.1).
#[derive(Debug, Clone, Default, Serialize)]
pub struct RunMeta {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_sha: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Summary {
    pub add: i64,
    pub change: i64,
    pub destroy: i64,
}

#[derive(Debug, Serialize)]
pub struct ShowReport {
    pub run: RunMeta,
    pub summary: Summary,
    /// True when only the plan-record summary was available (R5.6).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub degraded: bool,
    pub resource_changes: Vec<ResourceChangeReport>,
    pub output_changes: Vec<OutputChangeReport>,
}

#[derive(Debug, Serialize)]
pub struct ResourceChangeReport {
    pub address: String,
    /// create | update | replace | delete | read
    pub action: String,
    /// Attribute names whose change forces the replacement (R5.2).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub replace_forced_by: Vec<String>,
    pub attributes: Vec<AttributeChange>,
}

/// One attribute with already-redacted values: either real JSON or the
/// `(sensitive)` / `(known after apply)` marker strings.
#[derive(Debug, Serialize)]
pub struct AttributeChange {
    pub name: String,
    pub before: Value,
    pub after: Value,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub forces_replacement: bool,
}

#[derive(Debug, Serialize)]
pub struct OutputChangeReport {
    pub name: String,
    pub action: String,
    pub before: Value,
    pub after: Value,
}

/// A truthy sensitivity/unknown mask: `true`, or a non-empty container
/// (any nested sensitivity redacts the whole attribute — conservative
/// by design).
fn truthy_mask(mask: Option<&Value>) -> bool {
    match mask {
        Some(Value::Bool(b)) => *b,
        Some(Value::Object(m)) => !m.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        _ => false,
    }
}

fn classify_actions(actions: &[&str]) -> &'static str {
    match actions {
        ["create"] => "create",
        ["update"] => "update",
        ["delete"] => "delete",
        ["read"] => "read",
        ["delete", "create"] | ["create", "delete"] => "replace",
        _ => "no-op",
    }
}

/// Build the redacted report from full plan JSON.
pub fn build_report(run: RunMeta, plan: &Value) -> ShowReport {
    let mut summary = Summary::default();
    let mut changes = Vec::new();

    for rc in plan
        .get("resource_changes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let change = rc.get("change").cloned().unwrap_or(Value::Null);
        let actions: Vec<&str> = change
            .get("actions")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let action = classify_actions(&actions);
        match action {
            "create" => summary.add += 1,
            "update" => summary.change += 1,
            "delete" => summary.destroy += 1,
            "replace" => {
                summary.add += 1;
                summary.destroy += 1;
            }
            _ => {}
        }
        if action == "no-op" {
            continue;
        }

        let replace_forced_by: Vec<String> = change
            .get("replace_paths")
            .and_then(Value::as_array)
            .map(|paths| {
                paths
                    .iter()
                    .filter_map(|p| p.as_array()?.first())
                    .map(path_step_to_string)
                    .collect()
            })
            .unwrap_or_default();

        changes.push(ResourceChangeReport {
            address: rc
                .get("address")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            action: action.to_string(),
            attributes: build_attributes(&change, action, &replace_forced_by),
            replace_forced_by,
        });
    }

    // Group by action in the R5.1 order.
    let order = ["create", "update", "replace", "delete", "read"];
    changes.sort_by_key(|c| order.iter().position(|a| *a == c.action).unwrap_or(99));

    let output_changes = plan
        .get("output_changes")
        .and_then(Value::as_object)
        .map(|outputs| {
            outputs
                .iter()
                .filter_map(|(name, oc)| build_output(name, oc))
                .collect()
        })
        .unwrap_or_default();

    ShowReport {
        run,
        summary,
        degraded: false,
        resource_changes: changes,
        output_changes,
    }
}

/// Build the degraded report from the plan-record summary (R5.6).
pub fn build_degraded(run: RunMeta, summary: &PlanRecordSummary) -> ShowReport {
    ShowReport {
        run,
        summary: Summary {
            add: summary.additions,
            change: summary.changes,
            destroy: summary.destructions,
        },
        degraded: true,
        resource_changes: Vec::new(),
        output_changes: Vec::new(),
    }
}

fn path_step_to_string(step: &Value) -> String {
    match step {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Redact one side of an attribute. `present` distinguishes an absent
/// side (create's before, delete's after) from a null value.
fn redacted_value(value: Option<&Value>, sensitive: bool, unknown: bool) -> Value {
    if unknown {
        return Value::String(UNKNOWN.into());
    }
    match value {
        None => Value::Null,
        Some(v) => {
            if sensitive {
                Value::String(SENSITIVE.into())
            } else {
                v.clone()
            }
        }
    }
}

fn build_attributes(
    change: &Value,
    action: &str,
    replace_forced_by: &[String],
) -> Vec<AttributeChange> {
    let before = change.get("before").and_then(Value::as_object);
    let after = change.get("after").and_then(Value::as_object);
    let after_unknown = change.get("after_unknown").and_then(Value::as_object);
    let before_sensitive = change.get("before_sensitive").and_then(Value::as_object);
    let after_sensitive = change.get("after_sensitive").and_then(Value::as_object);

    let mut keys: BTreeSet<&String> = BTreeSet::new();
    for map in [before, after, after_unknown].into_iter().flatten() {
        keys.extend(map.keys());
    }

    let mut attrs = Vec::new();
    for key in keys {
        let b = before.and_then(|m| m.get(key));
        let a = after.and_then(|m| m.get(key));
        let unknown = truthy_mask(after_unknown.and_then(|m| m.get(key)));
        let sensitive = truthy_mask(before_sensitive.and_then(|m| m.get(key)))
            || truthy_mask(after_sensitive.and_then(|m| m.get(key)));

        // Only changed attributes are listed for update/replace. A
        // sensitive attribute with equal underlying values is unchanged —
        // equality is computed here, inside the redaction boundary, and
        // the values never leave it.
        if (action == "update" || action == "replace") && !unknown && b == a {
            continue;
        }

        attrs.push(AttributeChange {
            name: key.clone(),
            before: redacted_value(b, sensitive, false),
            after: redacted_value(a, sensitive, unknown),
            forces_replacement: replace_forced_by.contains(key),
        });
    }
    attrs
}

fn build_output(name: &str, oc: &Value) -> Option<OutputChangeReport> {
    let actions: Vec<&str> = oc
        .get("actions")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let action = classify_actions(&actions);
    if action == "no-op" {
        return None;
    }
    let sensitive =
        truthy_mask(oc.get("before_sensitive")) || truthy_mask(oc.get("after_sensitive"));
    let unknown = truthy_mask(oc.get("after_unknown"));
    Some(OutputChangeReport {
        name: name.to_string(),
        action: action.to_string(),
        before: redacted_value(oc.get("before"), sensitive, false),
        after: redacted_value(oc.get("after"), sensitive, unknown),
    })
}

/// Render the report as human-readable text. Receives only redacted
/// values by construction.
pub fn render_text(report: &ShowReport) -> String {
    let mut out = String::new();
    let run = &report.run;
    out.push_str(&format!("Run {}\n", run.id));
    if let Some(ws) = &run.workspace {
        out.push_str(&format!("  Workspace: {ws}\n"));
    }
    out.push_str(&format!("  Status:    {}\n", run.status));
    if let Some(source) = &run.source {
        out.push_str(&format!("  Source:    {source}\n"));
    }
    if let Some(sha) = &run.commit_sha {
        out.push_str(&format!("  Commit:    {sha}\n"));
    }
    if let Some(msg) = &run.message {
        // Subject line only — full commit bodies would break the header
        // layout; --format json keeps the complete message.
        let subject = msg.lines().next().unwrap_or_default();
        out.push_str(&format!("  Message:   {subject}\n"));
    }
    out.push('\n');
    out.push_str(&format!(
        "Plan: {} to add, {} to change, {} to destroy.{}\n",
        report.summary.add,
        report.summary.change,
        report.summary.destroy,
        if report.degraded {
            " (summary only)"
        } else {
            ""
        }
    ));

    if !report.resource_changes.is_empty() {
        out.push_str("\nResource changes:\n");
        for rc in &report.resource_changes {
            let symbol = match rc.action.as_str() {
                "create" => "+",
                "update" => "~",
                "replace" => "±",
                "delete" => "-",
                _ => " ",
            };
            out.push_str(&format!("\n  {symbol} {} {}", rc.action, rc.address));
            if !rc.replace_forced_by.is_empty() {
                out.push_str(&format!(
                    "  (forced by: {})",
                    rc.replace_forced_by.join(", ")
                ));
            }
            out.push('\n');
            for attr in &rc.attributes {
                out.push_str(&render_attribute(&rc.action, attr));
            }
        }
    }

    if !report.output_changes.is_empty() {
        out.push_str("\nOutput changes:\n");
        for oc in &report.output_changes {
            match oc.action.as_str() {
                "create" => {
                    out.push_str(&format!("  + {} = {}\n", oc.name, compact(&oc.after)));
                }
                "delete" => {
                    out.push_str(&format!("  - {} = {}\n", oc.name, compact(&oc.before)));
                }
                _ => {
                    out.push_str(&format!(
                        "  ~ {}: {} -> {}\n",
                        oc.name,
                        compact(&oc.before),
                        compact(&oc.after)
                    ));
                }
            }
        }
    }
    out
}

fn render_attribute(action: &str, attr: &AttributeChange) -> String {
    let force = if attr.forces_replacement {
        "  # forces replacement"
    } else {
        ""
    };
    match action {
        "create" => format!("      {} = {}{force}\n", attr.name, compact(&attr.after)),
        "delete" => format!("      {} = {}{force}\n", attr.name, compact(&attr.before)),
        _ => format!(
            "      {}: {} -> {}{force}\n",
            attr.name,
            compact(&attr.before),
            compact(&attr.after)
        ),
    }
}

/// Compact one-line rendering of an already-redacted value. Marker
/// strings print bare (no quotes) so they read as annotations.
fn compact(value: &Value) -> String {
    match value {
        Value::String(s) if s == SENSITIVE || s == UNKNOWN => s.clone(),
        other => serde_json::to_string(other).unwrap_or_else(|_| "null".into()),
    }
}
