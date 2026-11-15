//! Microsoft sign-in for OneDrive: OAuth 2.0 with PKCE and a one-shot loopback redirect, like
//! [`crate::oauth`] for Google. It's a public client (a desktop app), so there's no client secret.

use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;

use crate::oauth::{Tokens, catch_redirect, now_secs, random_token};

/// Files in the user's OneDrive (Astralis only touches its own folder, but shared vault folders
/// live elsewhere, so the app-folder-only permission isn't enough), and a refresh token.
pub const SCOPE: &str = "Files.ReadWrite offline_access User.Read";
const AUTH_URL: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/authorize";
const TOKEN_URL: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/token";
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
}

/// The consent URL.
pub fn auth_url(client_id: &str, redirect: &str, challenge: &str, state: &str) -> Result<String> {
    let params = [
        ("client_id", client_id),
        ("response_type", "code"),
        ("redirect_uri", redirect),
        ("response_mode", "query"),
        ("scope", SCOPE),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("state", state),
        ("prompt", "select_account"),
    ];
    Ok(reqwest::Url::parse_with_params(AUTH_URL, &params)?.to_string())
}

/// Runs the browser sign-in. `open` opens a URL in the browser.
pub async fn sign_in(http: &reqwest::Client, client_id: &str, open: impl FnOnce(&str) -> Result<()>) -> Result<Tokens> {
    if client_id.trim().is_empty() {
        bail!("OneDrive isn't set up yet: add the Microsoft app's client ID in Settings first");
    }
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.context("couldn't open a local port for sign-in")?;
    // Microsoft wants the name `localhost` for loopback redirects (any port).
    let redirect = format!("http://localhost:{}", listener.local_addr()?.port());
    let verifier = random_token()?;
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = random_token()?;
    open(&auth_url(client_id, &redirect, &challenge, &state)?)?;

    let query = tokio::time::timeout(SIGN_IN_TIMEOUT, catch_redirect(&listener)).await.map_err(|_| anyhow!("sign-in timed out"))??;
    let get = |k: &str| query.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
    if get("state").as_deref() != Some(state.as_str()) {
        bail!("sign-in response didn't match (state)");
    }
    if let Some(err) = get("error") {
        bail!(if err == "access_denied" {
            "sign-in was cancelled".to_string()
        } else {
            format!("Microsoft said: {}", get("error_description").unwrap_or(err))
        });
    }
    let code = get("code").ok_or_else(|| anyhow!("Microsoft didn't return a sign-in code"))?;
    // On a phone the app waits in the background and its network is blocked until you return:
    // keep trying (the code stays valid for minutes).
    let mut tries = 0;
    let resp = loop {
        let sent = http
            .post(TOKEN_URL)
            .form(&[
                ("client_id", client_id),
                ("scope", SCOPE),
                ("code", code.as_str()),
                ("code_verifier", verifier.as_str()),
                ("grant_type", "authorization_code"),
                ("redirect_uri", redirect.as_str()),
            ])
            .send()
            .await;
        match sent {
            Ok(r) => break r,
            Err(e) if tries < 60 && (e.is_connect() || e.is_timeout() || e.is_request()) => {
                tries += 1;
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Err(e) => return Err(e.into()),
        }
    };
    token_response(resp, None).await
}

/// A new access token from the refresh token.
pub async fn refresh(http: &reqwest::Client, client_id: &str, refresh_token: &str) -> Result<Tokens> {
    let resp = http
        .post(TOKEN_URL)
        .form(&[("client_id", client_id), ("scope", SCOPE), ("refresh_token", refresh_token), ("grant_type", "refresh_token")])
        .send()
        .await?;
    token_response(resp, Some(refresh_token)).await
}

async fn token_response(resp: reqwest::Response, keep_refresh: Option<&str>) -> Result<Tokens> {
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if text.contains("invalid_grant") {
            bail!("OneDrive sign-in expired or was revoked; connect OneDrive again");
        }
        bail!("Microsoft token request failed ({status}): {}", text.chars().take(300).collect::<String>());
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
    fn consent_url_uses_pkce_the_files_scope_and_a_localhost_redirect() {
        let url = auth_url("abc-123", "http://localhost:5555", "CHAL", "ST").unwrap();
        let parsed = reqwest::Url::parse(&url).unwrap();
        assert_eq!(parsed.host_str(), Some("login.microsoftonline.com"));
        let q: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        assert_eq!(q["scope"], SCOPE);
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["redirect_uri"], "http://localhost:5555");
        assert!(!q.contains_key("client_secret"));
    }
}
