//! J1.2: wiremock coverage for the JSON:API client — pagination, 429
//! retry honoring Retry-After, status→error mapping, and the
//! no-automatic-redirect policy.

use serde_json::json;
use tfrm_core::client::Client;
use wiremock::matchers::{bearer_token, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client(server: &MockServer) -> Client {
    Client::new(&server.uri(), "test-token".into()).unwrap()
}

#[tokio::test]
async fn paginates_across_three_pages() {
    let server = MockServer::start().await;
    for page in 1..=3u64 {
        let next = if page < 3 {
            json!(page + 1)
        } else {
            json!(null)
        };
        Mock::given(method("GET"))
            .and(path("/api/v2/organizations/acme/workspaces"))
            .and(query_param("page[number]", page.to_string()))
            .and(query_param("page[size]", "20"))
            .and(bearer_token("test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [
                    {"id": format!("ws-{page}a"), "type": "workspaces"},
                    {"id": format!("ws-{page}b"), "type": "workspaces"},
                ],
                "meta": {"pagination": {
                    "current-page": page,
                    "next-page": next,
                    "total-pages": 3
                }}
            })))
            .expect(1)
            .mount(&server)
            .await;
    }

    let (data, _included) = client(&server)
        .get_paginated("/api/v2/organizations/acme/workspaces", &[], 20, None)
        .await
        .unwrap();
    let ids: Vec<&str> = data.iter().map(|d| d["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["ws-1a", "ws-1b", "ws-2a", "ws-2b", "ws-3a", "ws-3b"]);
}

#[tokio::test]
async fn retries_429_honoring_retry_after() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/ping"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("Retry-After", "1")
                .set_body_json(json!({"errors": [{"title": "rate limited"}]})),
        )
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/ping"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": {"ok": true}})))
        .expect(1)
        .mount(&server)
        .await;

    let started = std::time::Instant::now();
    let doc = client(&server).get_json("/api/v2/ping").await.unwrap();
    assert!(doc["data"]["ok"].as_bool().unwrap());
    assert!(
        started.elapsed() >= std::time::Duration::from_secs(1),
        "retry did not wait for Retry-After: {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn rate_limit_gives_up_after_three_retries() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/ping"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
        // 1 initial + 3 retries
        .expect(4)
        .mount(&server)
        .await;

    let err = client(&server).get_json("/api/v2/ping").await.unwrap_err();
    assert_eq!(err.exit_code(), 1);
    let msg = err.to_string();
    assert!(msg.contains("429") && msg.contains("3 retries"), "{msg}");
}

#[tokio::test]
async fn status_to_error_mapping() {
    let server = MockServer::start().await;
    for (status, detail) in [
        (401u16, "unauthorized"),
        (403, "forbidden"),
        (404, "not found"),
        (409, "conflict"),
        (500, "server error"),
    ] {
        Mock::given(method("GET"))
            .and(path(format!("/api/v2/status/{status}")))
            .respond_with(ResponseTemplate::new(status).set_body_json(json!({
                "errors": [{"title": detail, "detail": format!("detail for {status}")}]
            })))
            .mount(&server)
            .await;
    }

    let c = client(&server);
    for (status, expected_exit) in [(401u16, 3), (403, 3), (404, 4), (409, 6), (500, 1)] {
        let err = c
            .get_json(&format!("/api/v2/status/{status}"))
            .await
            .unwrap_err();
        assert_eq!(err.exit_code(), expected_exit, "status {status}");
        let msg = err.to_string();
        assert!(msg.contains(&format!("HTTP {status}")), "{msg}");
        assert!(msg.contains(&format!("detail for {status}")), "{msg}");
        assert!(!msg.contains("test-token"), "token leaked: {msg}");
    }
}

#[tokio::test]
async fn never_follows_redirects_on_its_own() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v2/redirecting"))
        .respond_with(ResponseTemplate::new(307).insert_header(
            "Location",
            format!("{}/redirect-target", server.uri()).as_str(),
        ))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/redirect-target"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0) // the client must NOT arrive here by itself
        .mount(&server)
        .await;

    let resp = client(&server)
        .get_raw("/api/v2/redirecting")
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 307);
}
