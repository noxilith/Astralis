//! OneDrive (Microsoft Graph): the same small set of calls as Google Drive, behind the same traits.
//!
//! Differences that shape this file:
//! - OneDrive folders have no private "app properties". A vault folder's properties (which vault it
//!   is, which device owns it, invites) live in a small `.astralis-props.json` file inside it; the
//!   top-level `Astralis` folder (never shared) holds the vault keys the same way.
//! - Items shared with you live in the sharer's drive, so every id here is `"<drive id>/<item id>"`.

use std::collections::BTreeMap;

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use crate::drive::{CloudDrive, DriveFiles, FolderInfo, ROOT_NAME, RemoteFile};

const GRAPH: &str = "https://graph.microsoft.com/v1.0";
/// The file holding a folder's private properties (hidden from the sync file list).
pub const PROPS_FILE: &str = ".astralis-props.json";
/// Files up to this size go up in one request; bigger ones use an upload session.
const SIMPLE_MAX: usize = 3 * 1024 * 1024;
/// Upload session chunks must be a multiple of 320 KiB.
const CHUNK: usize = 320 * 1024 * 10;

/// OneDrive with an access token (callers refresh it).
pub struct OneDrive {
    http: reqwest::Client,
    token: String,
    /// This account's own drive, and its id as the creator of files.
    drive: String,
    user: String,
}

/// `"<drive>/<item>"` for an item.
fn join(drive: &str, item: &str) -> String {
    format!("{drive}/{item}")
}

/// Splits an id made by [`join`].
pub(crate) fn split(id: &str) -> Result<(&str, &str)> {
    match id.split_once('/') {
        Some((d, i)) if !d.is_empty() && !i.is_empty() => Ok((d, i)),
        _ => bail!("that isn't a OneDrive folder id: {id}"),
    }
}

fn item_url(id: &str) -> Result<String> {
    let (d, i) = split(id)?;
    Ok(format!("{GRAPH}/drives/{d}/items/{i}"))
}

/// Applies property changes (`None` removes) to a set of properties.
pub(crate) fn merge_props(mut props: BTreeMap<String, String>, changes: &BTreeMap<String, Option<String>>) -> BTreeMap<String, String> {
    for (k, v) in changes {
        match v {
            Some(v) => props.insert(k.clone(), v.clone()),
            None => props.remove(k),
        };
    }
    props
}

async fn check(resp: reqwest::Response) -> Result<reqwest::Response> {
    if resp.status().is_success() {
        return Ok(resp);
    }
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let msg = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(str::to_owned))
        .unwrap_or_else(|| text.chars().take(200).collect());
    if status.as_u16() == 404 {
        bail!("not found on OneDrive (or you don't have access to it): {msg}");
    }
    bail!("OneDrive error ({status}): {msg}")
}

impl OneDrive {
    /// Looks up this account's drive; the token must allow Files.ReadWrite.
    pub async fn connect(http: reqwest::Client, token: String) -> Result<Self> {
        let resp = http.get(format!("{GRAPH}/me/drive")).query(&[("$select", "id,owner")]).bearer_auth(&token).send().await?;
        let v: Value = check(resp).await?.json().await?;
        let drive = v["id"].as_str().ok_or_else(|| anyhow!("OneDrive didn't say which drive is yours"))?.to_owned();
        let user = v["owner"]["user"]["id"].as_str().unwrap_or_default().to_owned();
        Ok(Self { http, token, drive, user })
    }

    async fn get_json(&self, url: &str, query: &[(&str, &str)]) -> Result<Value> {
        let resp = self.http.get(url).query(query).bearer_auth(&self.token).send().await?;
        Ok(check(resp).await?.json().await?)
    }

    /// Every item directly inside a folder.
    async fn children(&self, folder: &str) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        let mut url = format!("{}/children", item_url(folder)?);
        let mut first = true;
        loop {
            let query: &[(&str, &str)] = if first { &[("$select", "id,name,file,folder,createdBy"), ("$top", "200")] } else { &[] };
            let v = self.get_json(&url, query).await?;
            out.extend(v["value"].as_array().cloned().unwrap_or_default());
            match v["@odata.nextLink"].as_str() {
                Some(next) => url = next.to_owned(),
                None => break,
            }
            first = false;
        }
        Ok(out)
    }

    async fn read_props(&self, folder: &str) -> Result<BTreeMap<String, String>> {
        let url = format!("{}:/{PROPS_FILE}:/content", item_url(folder)?);
        let resp = self.http.get(url).bearer_auth(&self.token).send().await?;
        if resp.status().as_u16() == 404 {
            return Ok(BTreeMap::new());
        }
        let bytes = check(resp).await?.bytes().await?;
        Ok(serde_json::from_slice(&bytes).unwrap_or_default())
    }

    async fn write_props(&self, folder: &str, props: &BTreeMap<String, String>) -> Result<()> {
        let url = format!("{}:/{PROPS_FILE}:/content", item_url(folder)?);
        let resp = self
            .http
            .put(url)
            .query(&[("@microsoft.graph.conflictBehavior", "replace")])
            .bearer_auth(&self.token)
            .header("Content-Type", "application/json")
            .body(serde_json::to_vec(props)?)
            .send()
            .await?;
        check(resp).await?;
        Ok(())
    }

    /// Uploads a big file in pieces.
    async fn upload_session(&self, folder: &str, name: &str, bytes: &[u8]) -> Result<String> {
        let url = format!("{}:/{name}:/createUploadSession", item_url(folder)?);
        let resp = self
            .http
            .post(url)
            .bearer_auth(&self.token)
            .json(&json!({ "item": { "@microsoft.graph.conflictBehavior": "replace" } }))
            .send()
            .await?;
        let v: Value = check(resp).await?.json().await?;
        let upload = v["uploadUrl"].as_str().ok_or_else(|| anyhow!("OneDrive didn't start the upload"))?.to_owned();
        let total = bytes.len();
        let mut start = 0;
        let mut last: Option<Value> = None;
        while start < total {
            let end = (start + CHUNK).min(total);
            // The upload URL is already authorised: no bearer token.
            let resp = self
                .http
                .put(&upload)
                .header("Content-Range", format!("bytes {start}-{}/{total}", end - 1))
                .body(bytes[start..end].to_vec())
                .send()
                .await?;
            let resp = check(resp).await?;
            if end == total {
                last = Some(resp.json().await?);
            }
            start = end;
        }
        let id = last.as_ref().and_then(|v| v["id"].as_str()).ok_or_else(|| anyhow!("OneDrive didn't confirm the upload"))?;
        Ok(join(split(folder)?.0, id))
    }
}

impl DriveFiles for OneDrive {
    async fn list(&self, folder: &str) -> Result<Vec<RemoteFile>> {
        let d = split(folder)?.0;
        Ok(self
            .children(folder)
            .await?
            .into_iter()
            .filter(|f| f.get("file").is_some())
            .filter_map(|f| {
                let name = f["name"].as_str()?.to_owned();
                if name.starts_with(".astralis") {
                    return None;
                }
                let creator = f["createdBy"]["user"]["id"].as_str().unwrap_or_default();
                Some(RemoteFile { id: join(d, f["id"].as_str()?), name, mine: !self.user.is_empty() && creator == self.user })
            })
            .collect())
    }

    async fn download(&self, id: &str) -> Result<Vec<u8>> {
        let resp = self.http.get(format!("{}/content", item_url(id)?)).bearer_auth(&self.token).send().await?;
        Ok(check(resp).await?.bytes().await?.to_vec())
    }

    async fn upload(&self, folder: &str, name: &str, bytes: Vec<u8>) -> Result<String> {
        if bytes.len() > SIMPLE_MAX {
            return self.upload_session(folder, name, &bytes).await;
        }
        let url = format!("{}:/{name}:/content", item_url(folder)?);
        let resp = self
            .http
            .put(url)
            .query(&[("@microsoft.graph.conflictBehavior", "replace"), ("$select", "id")])
            .bearer_auth(&self.token)
            .header("Content-Type", "application/octet-stream")
            .body(bytes)
            .send()
            .await?;
        let v: Value = check(resp).await?.json().await?;
        Ok(join(split(folder)?.0, v["id"].as_str().ok_or_else(|| anyhow!("OneDrive didn't confirm the upload"))?))
    }

    async fn owns(&self, folder: &str) -> Result<bool> {
        Ok(split(folder)?.0 == self.drive)
    }

    async fn delete(&self, id: &str) -> Result<()> {
        let resp = self.http.delete(item_url(id)?).bearer_auth(&self.token).send().await?;
        check(resp).await?;
        Ok(())
    }
}

impl CloudDrive for OneDrive {
    fn provider(&self) -> &'static str {
        astralis_core::cloud::ONEDRIVE
    }

    fn label(&self) -> &'static str {
        "OneDrive"
    }

    async fn email(&self) -> Result<String> {
        let v = self.get_json(&format!("{GRAPH}/me"), &[("$select", "mail,userPrincipalName")]).await?;
        v["mail"]
            .as_str()
            .or_else(|| v["userPrincipalName"].as_str())
            .map(str::to_owned)
            .ok_or_else(|| anyhow!("no email from Microsoft"))
    }

    async fn root_folder(&self) -> Result<String> {
        let url = format!("{GRAPH}/me/drive/root:/{ROOT_NAME}");
        let resp = self.http.get(&url).query(&[("$select", "id")]).bearer_auth(&self.token).send().await?;
        if resp.status().is_success() {
            let v: Value = resp.json().await?;
            return Ok(join(&self.drive, v["id"].as_str().ok_or_else(|| anyhow!("OneDrive sent a folder without an id"))?));
        }
        if resp.status().as_u16() != 404 {
            check(resp).await?;
        }
        let resp = self
            .http
            .post(format!("{GRAPH}/me/drive/root/children"))
            .bearer_auth(&self.token)
            .json(&json!({ "name": ROOT_NAME, "folder": {}, "@microsoft.graph.conflictBehavior": "fail" }))
            .send()
            .await?;
        if resp.status().as_u16() == 409 {
            // Another device made it a moment ago.
            let v = self.get_json(&url, &[("$select", "id")]).await?;
            return Ok(join(&self.drive, v["id"].as_str().ok_or_else(|| anyhow!("OneDrive sent a folder without an id"))?));
        }
        let v: Value = check(resp).await?.json().await?;
        Ok(join(&self.drive, v["id"].as_str().ok_or_else(|| anyhow!("OneDrive sent a folder without an id"))?))
    }

    async fn create_folder(&self, name: &str, parent: &str, props: &BTreeMap<String, String>) -> Result<String> {
        let resp = self
            .http
            .post(format!("{}/children", item_url(parent)?))
            .bearer_auth(&self.token)
            .json(&json!({ "name": name, "folder": {}, "@microsoft.graph.conflictBehavior": "rename" }))
            .send()
            .await?;
        let v: Value = check(resp).await?.json().await?;
        let id = join(split(parent)?.0, v["id"].as_str().ok_or_else(|| anyhow!("OneDrive sent a folder without an id"))?);
        if !props.is_empty() {
            self.write_props(&id, props).await?;
        }
        Ok(id)
    }

    async fn folder(&self, id: &str) -> Result<FolderInfo> {
        let v = self.get_json(&item_url(id)?, &[("$select", "id,name")]).await?;
        Ok(FolderInfo { id: id.to_owned(), name: v["name"].as_str().unwrap_or_default().to_owned(), props: self.read_props(id).await? })
    }

    async fn subfolders(&self, parent: &str) -> Result<Vec<FolderInfo>> {
        let d = split(parent)?.0;
        let mut out = Vec::new();
        for f in self.children(parent).await?.into_iter().filter(|f| f.get("folder").is_some()) {
            let (Some(item), Some(name)) = (f["id"].as_str(), f["name"].as_str()) else { continue };
            let id = join(d, item);
            let props = self.read_props(&id).await.unwrap_or_default();
            out.push(FolderInfo { id, name: name.to_owned(), props });
        }
        Ok(out)
    }

    async fn set_props(&self, id: &str, props: &BTreeMap<String, Option<String>>) -> Result<()> {
        let merged = merge_props(self.read_props(id).await?, props);
        self.write_props(id, &merged).await
    }

    async fn share_with(&self, id: &str, email: &str, message: &str) -> Result<()> {
        let resp = self
            .http
            .post(format!("{}/invite", item_url(id)?))
            .bearer_auth(&self.token)
            .json(&json!({
                "requireSignIn": true,
                "sendInvitation": true,
                "roles": ["write"],
                "recipients": [{ "email": email }],
                "message": message,
            }))
            .send()
            .await?;
        check(resp).await?;
        Ok(())
    }

    async fn unshare_with(&self, id: &str, email: &str) -> Result<()> {
        let url = format!("{}/permissions", item_url(id)?);
        let v = self.get_json(&url, &[]).await?;
        let email = email.trim().to_ascii_lowercase();
        let matches = |s: Option<&str>| s.map(str::to_ascii_lowercase).as_deref() == Some(email.as_str());
        for p in v["value"].as_array().into_iter().flatten() {
            let who = matches(p["invitation"]["email"].as_str())
                || matches(p["grantedToV2"]["user"]["email"].as_str())
                || matches(p["grantedTo"]["user"]["email"].as_str())
                || p["grantedToIdentitiesV2"].as_array().into_iter().flatten().any(|i| matches(i["user"]["email"].as_str()));
            if let (true, Some(pid)) = (who, p["id"].as_str()) {
                let resp = self.http.delete(format!("{url}/{pid}")).bearer_auth(&self.token).send().await?;
                check(resp).await?;
            }
        }
        Ok(())
    }

    async fn shared_folders(&self) -> Result<Vec<FolderInfo>> {
        let v = self.get_json(&format!("{GRAPH}/me/drive/sharedWithMe"), &[]).await?;
        let mut out = Vec::new();
        for item in v["value"].as_array().into_iter().flatten() {
            let r = &item["remoteItem"];
            if r.get("folder").is_none() {
                continue;
            }
            let (Some(drive), Some(rid)) = (r["parentReference"]["driveId"].as_str(), r["id"].as_str()) else { continue };
            // Only folders that carry Astralis properties are vaults.
            if let Ok(f) = self.folder(&join(drive, rid)).await
                && f.props.contains_key("astralis")
            {
                out.push(f);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_carry_the_drive_and_the_item() {
        assert_eq!(split("DRIVE1/ITEM!7").unwrap(), ("DRIVE1", "ITEM!7"));
        assert!(split("nodrive").is_err());
        assert!(split("/x").is_err());
        assert_eq!(item_url("D/I").unwrap(), "https://graph.microsoft.com/v1.0/drives/D/items/I");
    }

    #[test]
    fn property_changes_set_and_remove() {
        let start = BTreeMap::from([("a".to_string(), "1".to_string()), ("b".to_string(), "2".to_string())]);
        let changes = BTreeMap::from([("b".to_string(), None), ("c".to_string(), Some("3".to_string()))]);
        let out = merge_props(start, &changes);
        assert_eq!(out, BTreeMap::from([("a".to_string(), "1".to_string()), ("c".to_string(), "3".to_string())]));
    }
}
