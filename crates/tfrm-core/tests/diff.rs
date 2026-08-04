//! J2.5: plan-pair diff — the four categories, the R6.4 sensitive rule
//! with the sentinel invariant, identical plans, and the latest-applied
//! resolver hitting the right endpoint.

mod common;

use common::{assert_no_sentinel, fixture};
use serde_json::json;
use tfrm_core::client::Client;
use tfrm_core::diff::{self, SENSITIVE_DIFFERS};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn resources_only_in_one_plan_are_categorized() {
    // create.json changes aws_s3_bucket.assets; update.json changes
    // aws_instance.web — no overlap.
    let report = diff::diff_plans(
        "run-a",
        "run-b",
        &fixture("create.json"),
        &fixture("update.json"),
        false,
    );
    assert_eq!(report.only_in_a.len(), 1);
    assert_eq!(report.only_in_a[0].address, "aws_s3_bucket.assets");
    assert_eq!(report.only_in_a[0].action, "create");
    assert_eq!(report.only_in_b.len(), 1);
    assert_eq!(report.only_in_b[0].address, "aws_instance.web");
    assert!(report.differing.is_empty());
    assert!(report.has_differences());
}

#[test]
fn same_resource_with_differing_changes_shows_attribute_values() {
    let report = diff::diff_plans(
        "run-a",
        "run-b",
        &fixture("update.json"),
        &fixture("update-b.json"),
        false,
    );
    assert_eq!(report.differing.len(), 1);
    let d = &report.differing[0];
    assert_eq!(d.address, "aws_instance.web");
    assert_eq!(d.attributes.len(), 1);
    assert_eq!(d.attributes[0].name, "instance_type");
    assert_eq!(d.attributes[0].a, "t3.large");
    assert_eq!(d.attributes[0].b, "t3.2xlarge");

    let text = diff::render_text(&report);
    assert!(text.contains("instance_type"), "{text}");
    assert!(text.contains("t3.large"), "{text}");
    assert!(text.contains("t3.2xlarge"), "{text}");
}

#[test]
fn sensitive_equal_values_are_omitted() {
    // Passwords equal on both sides; instance_class differs. The password
    // attribute must be absent entirely.
    let report = diff::diff_plans(
        "run-a",
        "run-b",
        &fixture("sensitive.json"),
        &fixture("sensitive-equal.json"),
        false,
    );
    assert_eq!(report.differing.len(), 1);
    let d = &report.differing[0];
    let names: Vec<&str> = d.attributes.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["instance_class"], "password must be omitted");

    let text = diff::render_text(&report);
    assert!(!text.contains("password"), "{text}");
    assert_no_sentinel("sensitive-equal text", &text);
    assert_no_sentinel(
        "sensitive-equal json",
        &serde_json::to_string(&report).unwrap(),
    );
}

#[test]
fn sensitive_differing_values_show_marker_only() {
    let report = diff::diff_plans(
        "run-a",
        "run-b",
        &fixture("sensitive.json"),
        &fixture("sensitive-differ.json"),
        false,
    );
    assert_eq!(report.differing.len(), 1);
    let d = &report.differing[0];
    assert_eq!(d.attributes.len(), 1, "only the password differs");
    let attr = &d.attributes[0];
    assert_eq!(attr.name, "password");
    assert!(attr.sensitive_differs);
    assert!(attr.a.is_null() && attr.b.is_null());

    let text = diff::render_text(&report);
    assert!(
        text.contains(&format!("password: {SENSITIVE_DIFFERS}")),
        "{text}"
    );
    assert_no_sentinel("sensitive-differ text", &text);
    let json = serde_json::to_string(&report).unwrap();
    assert_no_sentinel("sensitive-differ json", &json);
    // The JSON entry must carry no value fields at all.
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    let entry = &parsed["differing"][0]["attributes"][0];
    assert!(
        entry.get("a").is_none() && entry.get("b").is_none(),
        "{entry}"
    );
}

#[test]
fn identical_plans_report_no_differences() {
    let report = diff::diff_plans(
        "run-a",
        "run-b",
        &fixture("update.json"),
        &fixture("update.json"),
        false,
    );
    assert!(!report.has_differences());
    assert_eq!(report.identical_count, 1);
    let text = diff::render_text(&report);
    assert!(text.contains("No differences."), "{text}");
}

#[test]
fn all_flag_lists_identical_addresses() {
    let report = diff::diff_plans(
        "run-a",
        "run-b",
        &fixture("update.json"),
        &fixture("update.json"),
        true,
    );
    assert_eq!(
        report.identical.as_deref(),
        Some(&["aws_instance.web".to_string()][..])
    );
}

/// R6.1: the resolver queries the applied-filtered runs listing, not the
/// deprecated latest-run or the any-status current-run relationship.
#[tokio::test]
async fn latest_applied_resolver_hits_the_applied_filter() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/workspaces/ws-two/runs"))
        .and(query_param("filter[status]", "applied"))
        .and(query_param("page[size]", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{"id": "run-applied", "type": "runs",
                       "attributes": {"status": "applied"}}]
        })))
        .expect(1)
        .mount(&server)
        .await;
    // The workspace endpoint (current-run relationship) must not be hit.
    Mock::given(method("GET"))
        .and(path("/api/v2/workspaces/ws-two"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    let client = Client::new(&server.uri(), "test-token".into()).unwrap();
    let id = diff::latest_applied_run(&client, "ws-two").await.unwrap();
    assert_eq!(id, "run-applied");
}

#[tokio::test]
async fn latest_applied_with_no_applied_runs_is_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/workspaces/ws-two/runs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": []})))
        .mount(&server)
        .await;

    let client = Client::new(&server.uri(), "test-token".into()).unwrap();
    let err = diff::latest_applied_run(&client, "ws-two")
        .await
        .unwrap_err();
    assert_eq!(err.exit_code(), 4);
}
