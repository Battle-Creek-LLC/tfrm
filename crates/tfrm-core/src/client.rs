//! JSON:API client for HCP Terraform / TFE (R8.3, R8.4).
//!
//! Bearer auth, transparent pagination, bounded 429 retry honoring
//! `Retry-After`, and typed error mapping. Redirects are never followed
//! automatically — R5.6's plan-JSON fetch handles its own 307 so it can
//! strip the Authorization header for the pre-signed URL.

use serde_json::Value;

use crate::error::{Error, Result};

/// Retries after a 429 before giving up (R8.4).
const MAX_RATE_LIMIT_RETRIES: u32 = 3;

pub struct Client {
    http: reqwest::Client,
    base: url::Url,
    token: String,
}

impl Client {
    /// `host` is a bare hostname like `app.terraform.io`; in tests a full
    /// `http://…` wiremock URL is accepted too.
    pub fn new(host: &str, token: String) -> Result<Self> {
        let base = if host.contains("://") {
            host.to_string()
        } else {
            format!("https://{host}")
        };
        let base = url::Url::parse(&base)
            .map_err(|e| Error::Usage(format!("invalid host {host}: {e}")))?;
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| Error::Other(format!("failed to build HTTP client: {e}")))?;
        Ok(Client { http, base, token })
    }

    pub fn host(&self) -> &str {
        self.base.host_str().unwrap_or("")
    }

    fn url(&self, path_and_query: &str) -> Result<url::Url> {
        self.base
            .join(path_and_query)
            .map_err(|e| Error::Other(format!("invalid URL path {path_and_query}: {e}")))
    }

    /// GET returning the raw response (no status mapping, no redirect
    /// following). J2.3 uses this for the plan-JSON 307 handshake.
    pub async fn get_raw(&self, path_and_query: &str) -> Result<reqwest::Response> {
        self.request_with_retry(reqwest::Method::GET, path_and_query, None)
            .await
    }

    /// GET a JSON:API document, mapping non-2xx statuses onto `Error`.
    pub async fn get_json(&self, path_and_query: &str) -> Result<Value> {
        let resp = self.get_raw(path_and_query).await?;
        Self::check_status(resp)
            .await?
            .json()
            .await
            .map_err(|e| Error::Other(format!("invalid JSON from {path_and_query}: {e}")))
    }

    /// POST a JSON:API document (or empty body when `body` is None),
    /// returning the response after status mapping. All run actions
    /// (apply/discard/cancel/override) go through here; a 409 maps to
    /// exit 6 per R7.9.
    pub async fn post(&self, path: &str, body: Option<Value>) -> Result<reqwest::Response> {
        let resp = self
            .request_with_retry(reqwest::Method::POST, path, body)
            .await?;
        Self::check_status(resp).await
    }

    /// GET every page of a JSON:API collection, following
    /// `meta.pagination.next-page`, and return the concatenated `data`
    /// arrays plus the first page's `included` entries (merged across
    /// pages). `max_items`, when set, stops paging once that many items
    /// are collected.
    pub async fn get_paginated(
        &self,
        path: &str,
        query: &[(&str, &str)],
        page_size: u32,
        max_items: Option<usize>,
    ) -> Result<(Vec<Value>, Vec<Value>)> {
        let mut data = Vec::new();
        let mut included = Vec::new();
        let mut page: u64 = 1;
        loop {
            let mut url = self.url(path)?;
            {
                let mut q = url.query_pairs_mut();
                for (k, v) in query {
                    q.append_pair(k, v);
                }
                q.append_pair("page[number]", &page.to_string());
                q.append_pair("page[size]", &page_size.to_string());
            }
            let doc = self.get_json(url.as_str()).await?;
            if let Some(items) = doc.get("data").and_then(|d| d.as_array()) {
                data.extend(items.iter().cloned());
            }
            if let Some(items) = doc.get("included").and_then(|d| d.as_array()) {
                included.extend(items.iter().cloned());
            }
            if let Some(max) = max_items {
                if data.len() >= max {
                    data.truncate(max);
                    break;
                }
            }
            match doc
                .pointer("/meta/pagination/next-page")
                .and_then(|p| p.as_u64())
            {
                Some(next) => page = next,
                None => break,
            }
        }
        Ok((data, included))
    }

    /// Issue one request, retrying on 429 per R8.4. Only the retry loop
    /// lives here — status mapping is the caller's business because the
    /// plan-JSON fetch must see 307/403 raw.
    async fn request_with_retry(
        &self,
        method: reqwest::Method,
        path_and_query: &str,
        body: Option<Value>,
    ) -> Result<reqwest::Response> {
        let url = self.url(path_and_query)?;
        let mut attempt: u32 = 0;
        loop {
            let mut req = self
                .http
                .request(method.clone(), url.clone())
                .bearer_auth(&self.token)
                .header("Content-Type", "application/vnd.api+json");
            if let Some(b) = &body {
                req = req.json(b);
            }
            // reqwest::Error Display can embed the URL but never headers,
            // so the R8.3 invariant holds on transport errors too.
            let resp = req
                .send()
                .await
                .map_err(|e| Error::Other(format!("request failed: {e}")))?;

            if resp.status().as_u16() == 429 && attempt < MAX_RATE_LIMIT_RETRIES {
                attempt += 1;
                let wait = resp
                    .headers()
                    .get("Retry-After")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.trim().parse::<f64>().ok())
                    .unwrap_or(1.0);
                tokio::time::sleep(std::time::Duration::from_secs_f64(wait.clamp(0.0, 60.0))).await;
                continue;
            }
            if resp.status().as_u16() == 429 {
                return Err(Error::api(
                    429,
                    format!("rate limited; giving up after {MAX_RATE_LIMIT_RETRIES} retries"),
                ));
            }
            return Ok(resp);
        }
    }

    /// Map a non-2xx response onto `Error::Api` with the TFC error detail
    /// (R8.3). Consumes the response body on error.
    pub async fn check_status(resp: reqwest::Response) -> Result<reqwest::Response> {
        let status = resp.status();
        if status.is_success() || status.is_redirection() {
            return Ok(resp);
        }
        let code = status.as_u16();
        let body: Option<Value> = resp.json().await.ok();
        Err(Error::api(code, Self::error_detail(code, body.as_ref())))
    }

    /// Extract the JSON:API error detail: joined `title: detail` pairs, or
    /// the HTTP reason phrase when the body has none.
    fn error_detail(status: u16, body: Option<&Value>) -> String {
        let from_body = body
            .and_then(|b| b.get("errors"))
            .and_then(|e| e.as_array())
            .map(|errors| {
                errors
                    .iter()
                    .filter_map(|e| {
                        let title = e.get("title").and_then(|t| t.as_str());
                        let detail = e.get("detail").and_then(|d| d.as_str());
                        match (title, detail) {
                            (Some(t), Some(d)) => Some(format!("{t}: {d}")),
                            (Some(t), None) => Some(t.to_string()),
                            (None, Some(d)) => Some(d.to_string()),
                            (None, None) => None,
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("; ")
            })
            .filter(|s| !s.is_empty());
        from_body.unwrap_or_else(|| {
            reqwest::StatusCode::from_u16(status)
                .ok()
                .and_then(|s| s.canonical_reason())
                .unwrap_or("error")
                .to_string()
        })
    }
}
