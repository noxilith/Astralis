//! The drive a cloud vault lives on, chosen when it's used: lets the app and sync work with any
//! provider through one type.

use std::collections::BTreeMap;

use anyhow::Result;

use crate::drive::{CloudDrive, DriveFiles, FolderInfo, GoogleDrive, RemoteFile};
use crate::onedrive::OneDrive;

pub enum AnyDrive {
    Google(GoogleDrive),
    One(OneDrive),
}

impl DriveFiles for AnyDrive {
    async fn list(&self, folder: &str) -> Result<Vec<RemoteFile>> {
        match self {
            Self::Google(d) => d.list(folder).await,
            Self::One(d) => d.list(folder).await,
        }
    }
    async fn download(&self, id: &str) -> Result<Vec<u8>> {
        match self {
            Self::Google(d) => d.download(id).await,
            Self::One(d) => d.download(id).await,
        }
    }
    async fn upload(&self, folder: &str, name: &str, bytes: Vec<u8>) -> Result<String> {
        match self {
            Self::Google(d) => d.upload(folder, name, bytes).await,
            Self::One(d) => d.upload(folder, name, bytes).await,
        }
    }
    async fn owns(&self, folder: &str) -> Result<bool> {
        match self {
            Self::Google(d) => d.owns(folder).await,
            Self::One(d) => d.owns(folder).await,
        }
    }
    async fn delete(&self, id: &str) -> Result<()> {
        match self {
            Self::Google(d) => DriveFiles::delete(d, id).await,
            Self::One(d) => DriveFiles::delete(d, id).await,
        }
    }
}

impl CloudDrive for AnyDrive {
    fn provider(&self) -> &'static str {
        match self {
            Self::Google(d) => d.provider(),
            Self::One(d) => d.provider(),
        }
    }
    fn label(&self) -> &'static str {
        match self {
            Self::Google(d) => d.label(),
            Self::One(d) => d.label(),
        }
    }
    async fn email(&self) -> Result<String> {
        match self {
            Self::Google(d) => CloudDrive::email(d).await,
            Self::One(d) => CloudDrive::email(d).await,
        }
    }
    async fn root_folder(&self) -> Result<String> {
        match self {
            Self::Google(d) => CloudDrive::root_folder(d).await,
            Self::One(d) => CloudDrive::root_folder(d).await,
        }
    }
    async fn create_folder(&self, name: &str, parent: &str, props: &BTreeMap<String, String>) -> Result<String> {
        match self {
            Self::Google(d) => CloudDrive::create_folder(d, name, parent, props).await,
            Self::One(d) => CloudDrive::create_folder(d, name, parent, props).await,
        }
    }
    async fn folder(&self, id: &str) -> Result<FolderInfo> {
        match self {
            Self::Google(d) => CloudDrive::folder(d, id).await,
            Self::One(d) => CloudDrive::folder(d, id).await,
        }
    }
    async fn subfolders(&self, parent: &str) -> Result<Vec<FolderInfo>> {
        match self {
            Self::Google(d) => CloudDrive::subfolders(d, parent).await,
            Self::One(d) => CloudDrive::subfolders(d, parent).await,
        }
    }
    async fn set_props(&self, id: &str, props: &BTreeMap<String, Option<String>>) -> Result<()> {
        match self {
            Self::Google(d) => CloudDrive::set_props(d, id, props).await,
            Self::One(d) => CloudDrive::set_props(d, id, props).await,
        }
    }
    async fn share_with(&self, id: &str, email: &str, message: &str) -> Result<()> {
        match self {
            Self::Google(d) => CloudDrive::share_with(d, id, email, message).await,
            Self::One(d) => CloudDrive::share_with(d, id, email, message).await,
        }
    }
    async fn unshare_with(&self, id: &str, email: &str) -> Result<()> {
        match self {
            Self::Google(d) => CloudDrive::unshare_with(d, id, email).await,
            Self::One(d) => CloudDrive::unshare_with(d, id, email).await,
        }
    }
    async fn shared_folders(&self) -> Result<Vec<FolderInfo>> {
        match self {
            Self::Google(d) => CloudDrive::shared_folders(d).await,
            Self::One(d) => CloudDrive::shared_folders(d).await,
        }
    }
}
