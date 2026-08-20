//! Resolving the secret an account authenticates with.
//!
//! Password accounts hand IMAP and SMTP the password straight from the
//! keychain. OAuth2 accounts hand them a short-lived access token instead,
//! which this module keeps fresh: the refresh token lives in the keychain
//! next to the passwords, the access token only ever in memory.

use crate::accounts;
use crate::db::Db;
use crate::error::{AppError, Result};
use crate::oauth::{self, Tokens};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Manager};

/// Settings key holding the Entra application (client) ID. The user pastes it
/// in Settings - every installation registers its own application, because a
/// desktop program has nowhere to hide a shared client secret.
pub const CLIENT_ID_KEY: &str = "oauth_client_id";

/// What a connection authenticates with.
pub enum Secret {
    Password(String),
    OAuth(String),
}

/// Access tokens, keyed by address. Deliberately not persisted - they expire
/// within the hour, so keeping them across restarts would buy nothing.
fn cache() -> &'static Mutex<HashMap<String, Tokens>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Tokens>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn client_id(app: &AppHandle) -> Result<String> {
    let db = app.state::<Db>();
    let conn = db.0.lock().unwrap();
    let stored: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [CLIENT_ID_KEY],
            |r| r.get(0),
        )
        .ok();
    match stored {
        Some(id) if !id.trim().is_empty() => Ok(id.trim().to_string()),
        _ => Err(AppError::Other(
            "no Microsoft application ID set - paste one in Settings".into(),
        )),
    }
}

/// A valid access token for the account, refreshing it if the cached one is
/// stale. Microsoft usually returns a new refresh token alongside, so we write
/// it back to the keychain - otherwise the chain would eventually break.
pub async fn access_token(app: &AppHandle, email: &str) -> Result<String> {
    // Short lock, released before the network call.
    if let Some(token) = {
        let cache = cache().lock().unwrap();
        cache
            .get(email)
            .filter(|t| t.is_fresh())
            .map(|t| t.access_token.clone())
    } {
        return Ok(token);
    }

    let client_id = client_id(app)?;
    let refresh_token = accounts::get_refresh_token(email)?;
    let tokens = oauth::refresh(&client_id, &refresh_token).await?;
    accounts::store_refresh_token(email, &tokens.refresh_token)?;

    let access = tokens.access_token.clone();
    cache().lock().unwrap().insert(email.to_string(), tokens);
    Ok(access)
}

/// Remembers a freshly issued set of tokens: refresh token to the keychain,
/// access token to the in-memory cache. Used right after signing in.
pub fn remember(email: &str, tokens: Tokens) -> Result<()> {
    accounts::store_refresh_token(email, &tokens.refresh_token)?;
    cache().lock().unwrap().insert(email.to_string(), tokens);
    Ok(())
}

pub fn forget(email: &str) {
    cache().lock().unwrap().remove(email);
}

/// What this account should authenticate with, whichever kind it is.
pub async fn secret_for(app: &AppHandle, email: &str) -> Result<Secret> {
    let kind: String = {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        conn.query_row(
            "SELECT auth_kind FROM accounts WHERE email = ?1",
            [email],
            |r| r.get(0),
        )?
    };
    if kind == "oauth2" {
        Ok(Secret::OAuth(access_token(app, email).await?))
    } else {
        Ok(Secret::Password(accounts::get_password(email)?))
    }
}
