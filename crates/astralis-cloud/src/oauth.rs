//! Google sign-in for desktop apps: OAuth 2.0 with PKCE and a one-shot loopback redirect.
//!
//! The browser opens Google's consent page; Google redirects to `http://127.0.0.1:<port>/`,
//! where we catch the code and exchange it for tokens. The same flow opens the Google Picker
//! (`trigger_onepick`), which returns the picked file/folder ids alongside the code.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Only files Astralis creates, or that the user picks for it.
pub const SCOPE: &str = "https://www.googleapis.com/auth/drive.file";
const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
/// How long to wait for the user to finish in the browser.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(300);

/// The user's own Google Cloud OAuth client ("Desktop app" type).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientConfig {
    pub client_id: String,
    /// Desktop clients have a "secret" that isn't secret (Google says so); it's still required.
    pub client_secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Unix seconds.
    pub expires_at: u64,
}

impl Tokens {
    pub fn fresh(&self) -> bool {
        now_secs() + 60 < self.expires_at
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
}

/// What came back from the browser.
#[derive(Debug)]
pub struct SignIn {
    pub tokens: Tokens,
    /// Files/folders picked in the Google Picker (empty for a plain sign-in).
    pub picked: Vec<String>,
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn random_token() -> Result<String> {
    let bytes = astralis_core::vault::random::<32>().map_err(|e| anyhow!("{e}"))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// The consent URL (`picker`: also show the Google Picker for choosing a folder).
pub fn auth_url(cfg: &ClientConfig, redirect: &str, challenge: &str, state: &str, picker: bool) -> Result<String> {
    let mut params = vec![
        ("client_id", cfg.client_id.as_str()),
        ("redirect_uri", redirect),
        ("response_type", "code"),
        ("scope", SCOPE),
        ("access_type", "offline"),
        ("prompt", "consent"),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("state", state),
    ];
    if picker {
        params.push(("trigger_onepick", "true"));
        params.push(("allow_folder_selection", "true"));
    }
    let url = reqwest::Url::parse_with_params(AUTH_URL, &params)?;
    Ok(url.to_string())
}

/// Runs the browser sign-in (optionally with the Picker). `open` opens a URL in the browser.
pub async fn sign_in(
    http: &reqwest::Client,
    cfg: &ClientConfig,
    picker: bool,
    open: impl FnOnce(&str) -> Result<()>,
) -> Result<SignIn> {
    if cfg.client_id.trim().is_empty() {
        bail!("Add your Google OAuth client ID in Settings first");
    }
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.context("couldn't open a local port for sign-in")?;
    let redirect = format!("http://127.0.0.1:{}", listener.local_addr()?.port());
    let verifier = random_token()?;
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = random_token()?;
    open(&auth_url(cfg, &redirect, &challenge, &state, picker)?)?;

    let query = tokio::time::timeout(SIGN_IN_TIMEOUT, catch_redirect(&listener))
        .await
        .map_err(|_| anyhow!("sign-in timed out"))??;
    let get = |k: &str| query.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
    if get("state").as_deref() != Some(state.as_str()) {
        bail!("sign-in response didn't match (state)");
    }
    if let Some(err) = get("error") {
        bail!(if err == "access_denied" { "sign-in was cancelled".to_string() } else { format!("Google said: {err}") });
    }
    let code = get("code").ok_or_else(|| anyhow!("Google didn't return a sign-in code"))?;
    let picked = get("picked_file_ids")
        .map(|s| s.split(',').filter(|x| !x.is_empty()).map(str::to_owned).collect())
        .unwrap_or_default();
    let tokens = exchange(http, cfg, &code, &verifier, &redirect).await?;
    Ok(SignIn { tokens, picked })
}

/// Waits for Google's redirect and answers it with a small "you can close this" page.
async fn catch_redirect(listener: &TcpListener) -> Result<Vec<(String, String)>> {
    loop {
        let (mut sock, _) = listener.accept().await?;
        let mut buf = vec![0u8; 16 * 1024];
        let n = sock.read(&mut buf).await?;
        let req = String::from_utf8_lossy(&buf[..n]);
        let path = req.lines().next().and_then(|l| l.split_whitespace().nth(1)).unwrap_or("/").to_owned();
        if !path.starts_with("/?") {
            // Browsers also ask for /favicon.ico and the like.
            let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
            continue;
        }
        let url = reqwest::Url::parse(&format!("http://localhost{path}"))?;
        let query: Vec<(String, String)> = url.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
        let ok = !query.iter().any(|(k, _)| k == "error");
        let body = format!(
            "<!doctype html><meta charset=utf-8><title>Astralis</title>\
             <body style=\"font-family:system-ui;background:#0f0f14;color:#e8e8f0;display:grid;place-items:center;height:100vh;margin:0\">\
             <div style=\"text-align:center\"><h2>{}</h2><p>You can close this tab and go back to Astralis.</p></div>",
            if ok { "Connected to Astralis" } else { "Sign-in cancelled" }
        );
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = sock.write_all(resp.as_bytes()).await;
        return Ok(query);
    }
}

async fn exchange(http: &reqwest::Client, cfg: &ClientConfig, code: &str, verifier: &str, redirect: &str) -> Result<Tokens> {
    let resp = http
        .post(TOKEN_URL)
        .form(&[
            ("client_id", cfg.client_id.as_str()),
            ("client_secret", cfg.client_secret.as_str()),
            ("code", code),
            ("code_verifier", verifier),
            ("grant_type", "authorization_code"),
            ("redirect_uri", redirect),
        ])
        .send()
        .await?;
    token_response(resp, None).await
}

/// A new access token from the refresh token.
pub async fn refresh(http: &reqwest::Client, cfg: &ClientConfig, refresh_token: &str) -> Result<Tokens> {
    let resp = http
        .post(TOKEN_URL)
        .form(&[
            ("client_id", cfg.client_id.as_str()),
            ("client_secret", cfg.client_secret.as_str()),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await?;
    token_response(resp, Some(refresh_token)).await
}

async fn token_response(resp: reqwest::Response, keep_refresh: Option<&str>) -> Result<Tokens> {
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if text.contains("invalid_grant") {
            bail!("Google sign-in expired or was revoked; connect Google Drive again");
        }
        bail!("Google token request failed ({status}): {}", text.chars().take(300).collect::<String>());
    }
    let t: TokenResponse = resp.json().await?;
    Ok(Tokens {
        access_token: t.access_token,
        refresh_token: t.refresh_token.or(keep_refresh.map(str::to_owned)),
        expires_at: now_secs() + t.expires_in.unwrap_or(3600),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consent_url_asks_only_for_drive_file_and_uses_pkce() {
        let cfg = ClientConfig { client_id: "abc.apps.googleusercontent.com".into(), client_secret: "s".into() };
        let url = auth_url(&cfg, "http://127.0.0.1:5555", "CHAL", "ST", false).unwrap();
        let parsed = reqwest::Url::parse(&url).unwrap();
        let q: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        assert_eq!(q["scope"], SCOPE);
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["redirect_uri"], "http://127.0.0.1:5555");
        assert!(!q.contains_key("trigger_onepick"));
        let picker = auth_url(&cfg, "http://127.0.0.1:5555", "CHAL", "ST", true).unwrap();
        assert!(picker.contains("trigger_onepick=true") && picker.contains("allow_folder_selection=true"));
    }

    #[tokio::test]
    async fn catches_the_redirect_and_ignores_other_requests() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let client = tokio::spawn(async move {
            let http = reqwest::Client::new();
            let _ = http.get(format!("http://127.0.0.1:{port}/favicon.ico")).send().await;
            http.get(format!("http://127.0.0.1:{port}/?code=XYZ&state=S&picked_file_ids=a,b")).send().await.unwrap().text().await.unwrap()
        });
        let q = catch_redirect(&listener).await.unwrap();
        assert!(q.contains(&("code".into(), "XYZ".into())));
        assert!(q.contains(&("picked_file_ids".into(), "a,b".into())));
        assert!(client.await.unwrap().contains("Connected to Astralis"));
    }
}
