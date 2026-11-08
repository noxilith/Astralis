//! Google Drive (v3) — just what cloud vaults need, behind a small trait so sync can be tested
//! against an in-memory drive.

use std::collections::BTreeMap;
use std::future::Future;

use anyhow::{Result, anyhow, bail};
use serde::Deserialize;
use serde_json::json;

const API: &str = "https://www.googleapis.com/drive/v3";
const UPLOAD: &str = "https://www.googleapis.com/upload/drive/v3/files";
const FOLDER: &str = "application/vnd.google-apps.folder";
/// The app's top-level folder in the user's Drive.
pub const ROOT_NAME: &str = "Astralis";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFile {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FolderInfo {
    pub id: String,
    pub name: String,
    /// Private to Astralis (other apps can't see them).
    pub props: BTreeMap<String, String>,
}

/// File operations sync needs. Implemented by [`GoogleDrive`] and, in tests, an in-memory drive.
pub trait DriveFiles: Send + Sync {
    /// Files in a folder that this app can read.
    fn list(&self, folder: &str) -> impl Future<Output = Result<Vec<RemoteFile>>> + Send;
    fn download(&self, id: &str) -> impl Future<Output = Result<Vec<u8>>> + Send;
    fn upload(&self, folder: &str, name: &str, bytes: Vec<u8>) -> impl Future<Output = Result<String>> + Send;
}

/// Google Drive with an access token (callers refresh it).
pub struct GoogleDrive {
    http: reqwest::Client,
    token: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileJson {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    app_properties: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListJson {
    #[serde(default)]
    files: Vec<FileJson>,
    next_page_token: Option<String>,
}

fn quote(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

impl GoogleDrive {
    pub fn new(http: reqwest::Client, access_token: String) -> Self {
        Self { http, token: access_token }
    }

    async fn check(resp: reqwest::Response) -> Result<reqwest::Response> {
        if resp.status().is_success() {
            return Ok(resp);
        }
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let msg = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v["error"]["message"].as_str().map(str::to_owned))
            .unwrap_or_else(|| text.chars().take(200).collect());
        if status.as_u16() == 404 {
            bail!("not found on Google Drive (or Astralis wasn't given access to it): {msg}");
        }
        bail!("Google Drive error ({status}): {msg}")
    }

    /// The signed-in account's email.
    pub async fn email(&self) -> Result<String> {
        let resp = self
            .http
            .get(format!("{API}/about"))
            .query(&[("fields", "user(emailAddress)")])
            .bearer_auth(&self.token)
            .send()
            .await?;
        let v: serde_json::Value = Self::check(resp).await?.json().await?;
        v["user"]["emailAddress"].as_str().map(str::to_owned).ok_or_else(|| anyhow!("no email from Google"))
    }

    /// The "Astralis" folder at the top of My Drive (created if needed).
    pub async fn root_folder(&self) -> Result<String> {
        let q = format!("name = '{ROOT_NAME}' and mimeType = '{FOLDER}' and 'root' in parents and trashed = false");
        let resp = self
            .http
            .get(format!("{API}/files"))
            .query(&[("q", q.as_str()), ("fields", "files(id,name)"), ("spaces", "drive")])
            .bearer_auth(&self.token)
            .send()
            .await?;
        let list: ListJson = Self::check(resp).await?.json().await?;
        if let Some(f) = list.files.into_iter().next() {
            return Ok(f.id);
        }
        self.create_folder(ROOT_NAME, "root", &BTreeMap::new()).await
    }

    pub async fn create_folder(&self, name: &str, parent: &str, props: &BTreeMap<String, String>) -> Result<String> {
        let resp = self
            .http
            .post(format!("{API}/files"))
            .query(&[("fields", "id")])
            .bearer_auth(&self.token)
            .json(&json!({ "name": name, "mimeType": FOLDER, "parents": [parent], "appProperties": props }))
            .send()
            .await?;
        let f: FileJson = Self::check(resp).await?.json().await?;
        Ok(f.id)
    }

    pub async fn folder(&self, id: &str) -> Result<FolderInfo> {
        let resp = self
            .http
            .get(format!("{API}/files/{id}"))
            .query(&[("fields", "id,name,appProperties"), ("supportsAllDrives", "true")])
            .bearer_auth(&self.token)
            .send()
            .await?;
        let f: FileJson = Self::check(resp).await?.json().await?;
        Ok(FolderInfo { id: f.id, name: f.name, props: f.app_properties })
    }

    /// Sets (or with `None`, removes) private app properties on a file or folder.
    pub async fn set_props(&self, id: &str, props: &BTreeMap<String, Option<String>>) -> Result<()> {
        let resp = self
            .http
            .patch(format!("{API}/files/{id}"))
            .query(&[("fields", "id")])
            .bearer_auth(&self.token)
            .json(&json!({ "appProperties": props }))
            .send()
            .await?;
        Self::check(resp).await?;
        Ok(())
    }

    pub async fn rename(&self, id: &str, name: &str) -> Result<()> {
        let resp = self
            .http
            .patch(format!("{API}/files/{id}"))
            .query(&[("fields", "id")])
            .bearer_auth(&self.token)
            .json(&json!({ "name": name }))
            .send()
            .await?;
        Self::check(resp).await?;
        Ok(())
    }

    /// Gives someone edit access to a folder; Google emails them.
    pub async fn share_with(&self, id: &str, email: &str, message: &str) -> Result<()> {
        let resp = self
            .http
            .post(format!("{API}/files/{id}/permissions"))
            .query(&[("sendNotificationEmail", "true"), ("emailMessage", message), ("fields", "id")])
            .bearer_auth(&self.token)
            .json(&json!({ "type": "user", "role": "writer", "emailAddress": email }))
            .send()
            .await?;
        Self::check(resp).await?;
        Ok(())
    }

    /// Removes someone's access to a folder (no error if they didn't have any).
    pub async fn unshare_with(&self, id: &str, email: &str) -> Result<()> {
        let resp = self
            .http
            .get(format!("{API}/files/{id}/permissions"))
            .query(&[("fields", "permissions(id,emailAddress)")])
            .bearer_auth(&self.token)
            .send()
            .await?;
        let v: serde_json::Value = Self::check(resp).await?.json().await?;
        let email = email.trim().to_ascii_lowercase();
        for p in v["permissions"].as_array().into_iter().flatten() {
            if p["emailAddress"].as_str().map(str::to_ascii_lowercase).as_deref() == Some(email.as_str())
                && let Some(pid) = p["id"].as_str()
            {
                let resp = self.http.delete(format!("{API}/files/{id}/permissions/{pid}")).bearer_auth(&self.token).send().await?;
                Self::check(resp).await?;
            }
        }
        Ok(())
    }

    /// Moves a file or folder to the trash.
    pub async fn trash(&self, id: &str) -> Result<()> {
        let resp = self
            .http
            .patch(format!("{API}/files/{id}"))
            .query(&[("fields", "id")])
            .bearer_auth(&self.token)
            .json(&json!({ "trashed": true }))
            .send()
            .await?;
        Self::check(resp).await?;
        Ok(())
    }
}

impl DriveFiles for GoogleDrive {
    async fn list(&self, folder: &str) -> Result<Vec<RemoteFile>> {
        let q = format!("'{}' in parents and trashed = false and mimeType != '{FOLDER}'", quote(folder));
        let mut out = Vec::new();
        let mut page: Option<String> = None;
        loop {
            let mut query = vec![
                ("q", q.clone()),
                ("fields", "files(id,name),nextPageToken".into()),
                ("pageSize", "1000".into()),
                ("orderBy", "createdTime".into()),
                ("supportsAllDrives", "true".into()),
                ("includeItemsFromAllDrives", "true".into()),
            ];
            if let Some(p) = &page {
                query.push(("pageToken", p.clone()));
            }
            let resp = self.http.get(format!("{API}/files")).query(&query).bearer_auth(&self.token).send().await?;
            let list: ListJson = Self::check(resp).await?.json().await?;
            out.extend(list.files.into_iter().map(|f| RemoteFile { id: f.id, name: f.name }));
            match list.next_page_token {
                Some(p) => page = Some(p),
                None => break,
            }
        }
        Ok(out)
    }

    async fn download(&self, id: &str) -> Result<Vec<u8>> {
        let resp = self
            .http
            .get(format!("{API}/files/{id}"))
            .query(&[("alt", "media"), ("supportsAllDrives", "true")])
            .bearer_auth(&self.token)
            .send()
            .await?;
        Ok(Self::check(resp).await?.bytes().await?.to_vec())
    }

    async fn upload(&self, folder: &str, name: &str, bytes: Vec<u8>) -> Result<String> {
        // multipart/related: JSON metadata, then the content.
        let boundary = format!("astralis-{}", uuid::Uuid::now_v7().simple());
        let meta = json!({ "name": name, "parents": [folder], "mimeType": "application/octet-stream" });
        let mut body = Vec::with_capacity(bytes.len() + 512);
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{meta}\r\n--{boundary}\r\nContent-Type: application/octet-stream\r\n\r\n")
                .as_bytes(),
        );
        body.extend_from_slice(&bytes);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        let resp = self
            .http
            .post(UPLOAD)
            .query(&[("uploadType", "multipart"), ("fields", "id"), ("supportsAllDrives", "true")])
            .bearer_auth(&self.token)
            .header("Content-Type", format!("multipart/related; boundary={boundary}"))
            .body(body)
            .send()
            .await?;
        let f: FileJson = Self::check(resp).await?.json().await?;
        Ok(f.id)
    }
}
