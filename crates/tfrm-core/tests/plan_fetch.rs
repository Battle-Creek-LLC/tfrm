//! J2.3: plan JSON fetch — 307 followed without Authorization, 403
//! fallback to the plan-record summary, and the sentinel invariant on
//! the fallback's serialized forms.

mod common;

use common::{assert_no_sentinel, fixture};
use serde_json::json;
use tfrm_core::client::Client;
use tfrm_core::plan::{self, PlanFetch};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn client(server: &MockServer) -> Client {
    Client::new(&server.uri(), "test-token".into()).unwrap()
}

/// Matches only requests carrying no Authorization header.
struct NoAuthHeader;

impl wiremock::Match for NoAuthHeader {
    fn matches(&self, request: &Request) -> bool {
        !request.headers.contains_key("authorization")
    }
}

#[tokio::test]
async fn follows_307_without_forwarding_authorization() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/runs/run-x/plan/json-output"))
        .respond_with(ResponseTemplate::new(307).insert_header(
            "Location",
            format!("{}/presigned/plan-abc", server.uri()).as_str(),
        ))
        .expect(1)
        .mount(&server)
        .await;
    // The pre-signed mock only matches when Authorization is absent, so a
    // forwarded bearer token fails the fetch (and the request count).
    Mock::given(method("GET"))
        .and(path("/presigned/plan-abc"))
        .and(NoAuthHeader)
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("sensitive.json")))
        .expect(1)
        .mount(&server)
        .await;

    let result = plan::fetch(&client(&server), "run-x").await.unwrap();
    match result {
        PlanFetch::Full(value) => {
            assert_eq!(value["format_version"], "1.2");
            assert_eq!(
                value["resource_changes"][0]["address"],
                "aws_db_instance.main"
            );
        }
        PlanFetch::Summary(_) => panic!("expected full plan"),
    }
}

#[tokio::test]
async fn forbidden_falls_back_to_plan_record_summary() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/runs/run-x/plan/json-output"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "errors": [{"status": "403", "title": "forbidden"}]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/runs/run-x/plan"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": {
                "id": "plan-abc",
                "type": "plans",
                "attributes": {
                    "resource-additions": 2,
                    "resource-changes": 1,
                    "resource-destructions": 3,
                    "log-read-url": format!("{}/presigned/log", server.uri())
                }
            }
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/presigned/log"))
        .and(NoAuthHeader)
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("Plan: 2 to add, 1 to change, 3 to destroy."),
        )
        .mount(&server)
        .await;

    let result = plan::fetch(&client(&server), "run-x").await.unwrap();
    match result {
        PlanFetch::Summary(summary) => {
            assert_eq!(summary.additions, 2);
            assert_eq!(summary.changes, 1);
            assert_eq!(summary.destructions, 3);
            assert!(summary.log.as_deref().unwrap().contains("2 to add"));
            // Sentinel invariant on every serialized form of the fallback.
            assert_no_sentinel("summary debug", &format!("{summary:?}"));
            assert_no_sentinel("summary json", &serde_json::to_string(&summary).unwrap());
        }
        PlanFetch::Full(_) => panic!("expected summary fallback"),
    }
}

#[tokio::test]
async fn missing_plan_maps_to_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/runs/run-x/plan/json-output"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "errors": [{"status": "404", "title": "not found"}]
        })))
        .mount(&server)
        .await;

    let err = plan::fetch(&client(&server), "run-x").await.unwrap_err();
    assert_eq!(err.exit_code(), 4);
}

/// The sensitive fixture really carries the sentinel — guards against the
/// fixture being "sanitized" away, which would blind every renderer test.
#[test]
fn sensitive_fixture_contains_the_sentinel() {
    let text = serde_json::to_string(&fixture("sensitive.json")).unwrap();
    assert!(text.contains(common::SENTINEL));
}
