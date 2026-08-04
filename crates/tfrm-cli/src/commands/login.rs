//! `tfrm login` — the interactive OAuth flow (R2b.1–R2b.5): localhost
//! callback listener, browser + printed URL, and the paste fallback,
//! racing whichever arrives first.

use tfrm_core::login::{self, OAuthConfig, Pkce};
use tfrm_core::{credfile, Error, Result};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};

pub async fn login(host: &str) -> Result<()> {
    let config = login::discover(host).await?;
    let pkce = login::generate_pkce();

    // Bind the advertised port range; exhaustion falls back to paste-only.
    let listener = bind_callback_listener(&config).await;
    let redirect_port = listener
        .as_ref()
        .and_then(|l| l.local_addr().ok().map(|a| a.port()))
        .unwrap_or(config.min_port);
    let redirect_uri = format!("http://localhost:{redirect_port}/login");
    if listener.is_none() {
        eprintln!(
            "note: no localhost port in {}-{} could be opened; paste the redirect URL or \
             code manually",
            config.min_port, config.max_port
        );
    }

    let authorize_url = login::authorize_url(&config, &pkce, &redirect_uri);
    eprintln!("Open this URL in your browser to authorize tfrm:\n\n  {authorize_url}\n");
    open_browser(&authorize_url);
    eprintln!("Waiting for the browser callback. You can also paste the full redirect URL");
    eprint!("or the authorization code here: ");

    let code = wait_for_code(listener, &pkce).await?;

    let token = login::exchange_code(&config, &code, &pkce.verifier, &redirect_uri).await?;
    let account = login::verify_token(host, &token).await?;

    let path = credfile::default_path()?;
    credfile::store(&path, host, &token)?;
    // R2.3: report success and provenance, never the token.
    println!("Logged in to {host} as {account}");
    println!("Token stored in {}", path.display());
    Ok(())
}

/// `tfrm logout` (R2b.6): remove the host's entry from
/// credentials.tfrc.json; no-op with a note when absent. `.terraformrc`
/// credentials blocks are never touched.
pub fn logout(host: &str) -> Result<()> {
    let path = credfile::default_path()?;
    if credfile::remove(&path, host)? {
        println!(
            "Logged out of {host} (token removed from {})",
            path.display()
        );
    } else {
        println!(
            "No stored token for {host} in {}; nothing to do",
            path.display()
        );
    }
    Ok(())
}

async fn bind_callback_listener(config: &OAuthConfig) -> Option<tokio::net::TcpListener> {
    for port in config.min_port..=config.max_port {
        if let Ok(listener) = tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
            return Some(listener);
        }
    }
    None
}

fn open_browser(url: &str) {
    if std::env::var_os("TFRM_NO_BROWSER").is_some() {
        return;
    }
    #[cfg(target_os = "macos")]
    let opener = "open";
    #[cfg(not(target_os = "macos"))]
    let opener = "xdg-open";
    let _ = std::process::Command::new(opener)
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// R2b.3: accept whichever arrives first — the browser callback or a
/// pasted redirect URL / bare code on stdin.
async fn wait_for_code(listener: Option<tokio::net::TcpListener>, pkce: &Pkce) -> Result<String> {
    let mut stdin = tokio::io::BufReader::new(tokio::io::stdin());
    let mut line = String::new();

    match listener {
        Some(listener) => {
            tokio::select! {
                callback = accept_callback(&listener, &pkce.state) => callback,
                read = stdin.read_line(&mut line) => {
                    read.map_err(|e| Error::Other(format!("cannot read input: {e}")))?;
                    parse_pasted(line.trim(), &pkce.state)
                }
            }
        }
        None => {
            stdin
                .read_line(&mut line)
                .await
                .map_err(|e| Error::Other(format!("cannot read input: {e}")))?;
            parse_pasted(line.trim(), &pkce.state)
        }
    }
}

/// Serve the localhost callback until a request carrying a valid state
/// and code arrives.
async fn accept_callback(listener: &tokio::net::TcpListener, state: &str) -> Result<String> {
    loop {
        let (mut stream, _) = listener
            .accept()
            .await
            .map_err(|e| Error::Other(format!("callback listener failed: {e}")))?;
        let mut buf = vec![0u8; 8192];
        let n = stream.read(&mut buf).await.unwrap_or(0);
        let request = String::from_utf8_lossy(&buf[..n]).to_string();

        let Some(query) = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|path| path.split_once('?').map(|(_, q)| q.to_string()))
        else {
            let _ = respond(&mut stream, 400, "Bad request").await;
            continue;
        };
        let params: std::collections::HashMap<String, String> =
            url::form_urlencoded::parse(query.as_bytes())
                .into_owned()
                .collect();

        if params.get("state").map(String::as_str) != Some(state) {
            let _ = respond(&mut stream, 400, "State mismatch; check the CLI").await;
            continue;
        }
        let Some(code) = params.get("code").filter(|c| !c.is_empty()) else {
            let _ = respond(&mut stream, 400, "Missing authorization code").await;
            continue;
        };
        let _ = respond(&mut stream, 200, "Login complete — return to the terminal.").await;
        return Ok(code.clone());
    }
}

async fn respond(stream: &mut tokio::net::TcpStream, status: u16, body: &str) -> Result<()> {
    let reason = if status == 200 { "OK" } else { "Bad Request" };
    let payload = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(payload.as_bytes())
        .await
        .map_err(|e| Error::Other(format!("cannot answer callback: {e}")))?;
    Ok(())
}

/// A pasted full redirect URL must echo our state (refused otherwise); a
/// bare code carries no state — accept it and note the skipped check.
fn parse_pasted(input: &str, state: &str) -> Result<String> {
    if input.is_empty() {
        return Err(Error::Other("no authorization code provided".into()));
    }
    if input.contains("://") {
        let url = url::Url::parse(input)
            .map_err(|e| Error::Usage(format!("cannot parse the pasted URL: {e}")))?;
        let params: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        if params.get("state").map(String::as_str) != Some(state) {
            return Err(Error::Auth(
                "the pasted URL's state does not match this login attempt; refusing the code \
                 (possible cross-request forgery or a stale URL)"
                    .into(),
            ));
        }
        return params
            .get("code")
            .filter(|c| !c.is_empty())
            .cloned()
            .ok_or_else(|| Error::Usage("the pasted URL carries no authorization code".into()));
    }
    eprintln!("note: a bare code carries no state, so the state check was skipped");
    Ok(input.to_string())
}
