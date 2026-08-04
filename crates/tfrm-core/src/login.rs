//! `tfrm login` protocol pieces (R2b.1–R2b.4): `login.v1` service
//! discovery, PKCE material, the code-for-token exchange, and token
//! verification. The interactive parts (listener, browser, prompt) live
//! in the CLI; everything here is testable without a terminal.

use base64::Engine;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

/// OAuth client configuration advertised by the host's
/// `/.well-known/terraform.json` under `login.v1`.
#[derive(Debug, Clone)]
pub struct OAuthConfig {
    pub client_id: String,
    pub authz_url: url::Url,
    pub token_url: url::Url,
    /// Inclusive localhost callback port range.
    pub min_port: u16,
    pub max_port: u16,
    pub scopes: Vec<String>,
}

fn base_url(host: &str) -> Result<url::Url> {
    let base = if host.contains("://") {
        host.to_string()
    } else {
        format!("https://{host}")
    };
    url::Url::parse(&base).map_err(|e| Error::Usage(format!("invalid host {host}: {e}")))
}

/// Discover the host's `login.v1` OAuth client config. Exit 4 when the
/// host does not advertise `login.v1` (R2b.1).
pub async fn discover(host: &str) -> Result<OAuthConfig> {
    let base = base_url(host)?;
    let well_known = base
        .join("/.well-known/terraform.json")
        .map_err(|e| Error::Other(format!("cannot build discovery URL: {e}")))?;
    let resp = reqwest::get(well_known.clone())
        .await
        .map_err(|e| Error::Other(format!("service discovery failed for {host}: {e}")))?;
    if !resp.status().is_success() {
        return Err(Error::NotFound(format!(
            "{host} does not answer service discovery ({}); is it a Terraform Enterprise / \
             HCP Terraform host?",
            resp.status()
        )));
    }
    let doc: Value = resp
        .json()
        .await
        .map_err(|e| Error::Other(format!("invalid discovery document from {host}: {e}")))?;

    let login = doc.get("login.v1").ok_or_else(|| {
        Error::NotFound(format!(
            "{host} does not advertise the login.v1 service; tfrm login is not supported \
             for this host"
        ))
    })?;

    let resolve = |field: &str| -> Result<url::Url> {
        let raw = login
            .get(field)
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Other(format!("discovery login.v1 is missing \"{field}\"")))?;
        well_known
            .join(raw)
            .map_err(|e| Error::Other(format!("invalid login.v1 {field} URL: {e}")))
    };

    let ports: Vec<u16> = login
        .get("ports")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_u64)
                .filter_map(|p| u16::try_from(p).ok())
                .collect()
        })
        .unwrap_or_default();
    let (min_port, max_port) = match ports.as_slice() {
        [min, max, ..] => (*min, *max),
        _ => (10000, 10010),
    };

    Ok(OAuthConfig {
        client_id: login
            .get("client")
            .and_then(Value::as_str)
            .unwrap_or("terraform-cli")
            .to_string(),
        authz_url: resolve("authz")?,
        token_url: resolve("token")?,
        min_port,
        max_port,
        scopes: login
            .get("scopes")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// PKCE S256 material plus the request state (R2b.2).
#[derive(Debug, Clone)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
    pub state: String,
}

pub fn generate_pkce() -> Pkce {
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let random = |n: usize| -> String {
        let bytes: Vec<u8> = (0..n).map(|_| rand::random::<u8>()).collect();
        engine.encode(bytes)
    };
    let verifier = random(32);
    let challenge = engine.encode(Sha256::digest(verifier.as_bytes()));
    Pkce {
        verifier,
        challenge,
        state: random(16),
    }
}

/// The full authorize URL the user's browser visits.
pub fn authorize_url(config: &OAuthConfig, pkce: &Pkce, redirect_uri: &str) -> String {
    let mut url = config.authz_url.clone();
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("client_id", &config.client_id);
        q.append_pair("code_challenge", &pkce.challenge);
        q.append_pair("code_challenge_method", "S256");
        q.append_pair("redirect_uri", redirect_uri);
        q.append_pair("response_type", "code");
        q.append_pair("state", &pkce.state);
        if !config.scopes.is_empty() {
            q.append_pair("scope", &config.scopes.join(" "));
        }
    }
    url.to_string()
}

/// Exchange the authorization code (with the PKCE verifier) for a token
/// as a public client (R2b.4).
pub async fn exchange_code(
    config: &OAuthConfig,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<String> {
    let client = reqwest::Client::new();
    let resp = client
        .post(config.token_url.clone())
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", config.client_id.as_str()),
            ("code", code),
            ("code_verifier", verifier),
            ("redirect_uri", redirect_uri),
        ])
        .send()
        .await
        .map_err(|e| Error::Other(format!("token exchange failed: {e}")))?;
    if !resp.status().is_success() {
        // The body may describe the OAuth error; it never contains our
        // token (we don't have one yet), so it is safe to surface.
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        return Err(Error::Auth(format!(
            "token exchange was refused (HTTP {status}): {body}"
        )));
    }
    let doc: Value = resp
        .json()
        .await
        .map_err(|e| Error::Other(format!("invalid token response: {e}")))?;
    doc.get("access_token")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| Error::Auth("token response carried no access_token".into()))
}

/// Verify a token against `/api/v2/account/details` and return the
/// account name (R2b.4).
pub async fn verify_token(host: &str, token: &str) -> Result<String> {
    let base = base_url(host)?;
    let url = base
        .join("/api/v2/account/details")
        .map_err(|e| Error::Other(format!("cannot build account URL: {e}")))?;
    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| Error::Other(format!("account verification failed: {e}")))?;
    if !resp.status().is_success() {
        return Err(Error::Auth(format!(
            "the new token failed verification against account/details (HTTP {})",
            resp.status().as_u16()
        )));
    }
    let doc: Value = resp
        .json()
        .await
        .map_err(|e| Error::Other(format!("invalid account response: {e}")))?;
    Ok(doc
        .pointer("/data/attributes/username")
        .or_else(|| doc.pointer("/data/attributes/name"))
        .and_then(Value::as_str)
        .unwrap_or("(unknown account)")
        .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_is_s256_of_verifier() {
        let pkce = generate_pkce();
        let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let expected = engine.encode(Sha256::digest(pkce.verifier.as_bytes()));
        assert_eq!(pkce.challenge, expected);
        assert_ne!(pkce.verifier, pkce.challenge);
        assert!(!pkce.state.is_empty());
    }

    #[test]
    fn authorize_url_carries_pkce_and_state() {
        let config = OAuthConfig {
            client_id: "terraform-cli".into(),
            authz_url: url::Url::parse("https://example.com/oauth/authorize").unwrap(),
            token_url: url::Url::parse("https://example.com/oauth/token").unwrap(),
            min_port: 10000,
            max_port: 10010,
            scopes: vec![],
        };
        let pkce = generate_pkce();
        let url = authorize_url(&config, &pkce, "http://localhost:10000/login");
        assert!(url.contains(&format!("code_challenge={}", pkce.challenge)));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains(&format!("state={}", pkce.state)));
        assert!(url.contains("response_type=code"));
    }
}
