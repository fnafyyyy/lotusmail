use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("błąd bazy danych: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("błąd magazynu haseł: {0}")]
    Keyring(#[from] keyring::Error),
    #[error("błąd we/wy: {0}")]
    Io(#[from] std::io::Error),
    #[error("błąd IMAP: {0}")]
    Imap(#[from] async_imap::error::Error),
    #[error("błąd TLS: {0}")]
    Tls(#[from] native_tls::Error),
    #[error("{0}")]
    Other(String),
}

impl Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

pub type Result<T> = std::result::Result<T, AppError>;
