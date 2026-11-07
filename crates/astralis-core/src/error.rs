use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0} not found: {1}")]
    NotFound(&'static str, Uuid),
    #[error("vault is locked")]
    VaultLocked(Uuid),
    #[error("wrong password")]
    WrongPassword,
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("encryption error: {0}")]
    Crypto(String),
    #[error("AI error: {0}")]
    Ai(String),
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

impl Error {
    pub(crate) fn invalid(msg: impl Into<String>) -> Self {
        Error::Invalid(msg.into())
    }
}

impl serde::Serialize for Error {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
