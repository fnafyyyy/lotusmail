//! Załączniki: pobieranie na żądanie z serwera IMAP, zapis na dysk i metadane
//! w bazie. Pliki trzymamy obok bazy (`attachments/<id wiadomości>/`), żeby
//! baza nie puchła, a otwieranie sprowadzało się do podania ścieżki systemowi.

use crate::db::Db;
use crate::error::{AppError, Result};
use futures_util::TryStreamExt;
use mail_parser::MimeHeaders;
use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub id: i64,
    pub name: String,
    pub mime: String,
    pub size: i64,
    pub path: String,
    pub is_inline: bool,
}

/// Nazwa pliku bezpieczna dla systemu plików Windows.
fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if r#"<>:"/\|?*"#.contains(c) || (c as u32) < 32 { '_' } else { c })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').to_string();
    if trimmed.is_empty() {
        "zalacznik".into()
    } else {
        trimmed.chars().take(120).collect()
    }
}

fn store_dir(app: &AppHandle, message_id: i64) -> Result<PathBuf> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Other(format!("katalog danych: {e}")))?
        .join("attachments")
        .join(message_id.to_string());
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn stored(app: &AppHandle, message_id: i64) -> Result<Vec<Attachment>> {
    let db = app.state::<Db>();
    let conn = db.0.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT id, name, mime, size, path, is_inline FROM attachments
         WHERE message_id = ?1 ORDER BY is_inline, id",
    )?;
    let rows = stmt.query_map([message_id], |r| {
        Ok(Attachment {
            id: r.get(0)?,
            name: r.get(1)?,
            mime: r.get(2)?,
            size: r.get(3)?,
            path: r.get(4)?,
            is_inline: r.get::<_, i64>(5)? != 0,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Załączniki wiadomości. Gdy nie ma ich jeszcze na dysku, dociągamy surową
/// wiadomość z serwera i rozpakowujemy - dzięki temu nie musimy przechowywać
/// wszystkich załączników z całej skrzynki.
pub async fn get(app: &AppHandle, message_id: i64) -> Result<Vec<Attachment>> {
    let existing = stored(app, message_id)?;
    if !existing.is_empty() {
        return Ok(existing);
    }

    let (uid, folder_name, account_id, has_attachments) = {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        conn.query_row(
            "SELECT m.uid, f.name, f.account_id, m.has_attachments
             FROM messages m JOIN folders f ON f.id = m.folder_id
             WHERE m.id = ?1",
            [message_id],
            |r| {
                Ok((
                    r.get::<_, u32>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)? != 0,
                ))
            },
        )?
    };
    if !has_attachments || uid == 0 {
        return Ok(vec![]);
    }

    let (email, login, host, port) = {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        conn.query_row(
            "SELECT email, login, imap_host, imap_port FROM accounts WHERE id = ?1",
            [account_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, u16>(3)?,
                ))
            },
        )?
    };
    let login = if login.is_empty() { email.clone() } else { login };
    let password = crate::accounts::get_password(&email)?;

    let mut session = crate::sync::connect_session(&host, port, &login, &password).await?;
    session.select(&folder_name).await?;
    let stream = session.uid_fetch(uid.to_string(), "(BODY.PEEK[])").await?;
    let fetched = stream.try_collect::<Vec<_>>().await?;
    session.logout().await.ok();

    let raw = fetched
        .first()
        .and_then(|f| f.body())
        .ok_or_else(|| AppError::Other("serwer nie zwrócił treści wiadomości".into()))?
        .to_vec();
    let parsed = mail_parser::MessageParser::default()
        .parse(&raw)
        .ok_or_else(|| AppError::Other("nie udało się rozpakować wiadomości".into()))?;

    let dir = store_dir(app, message_id)?;
    let mut result = Vec::new();
    for (index, part) in parsed.attachments().enumerate() {
        let mime = part
            .content_type()
            .map(|ct| match ct.subtype() {
                Some(st) => format!("{}/{}", ct.ctype(), st),
                None => ct.ctype().to_string(),
            })
            .unwrap_or_else(|| "application/octet-stream".into());
        // Obrazki osadzone w treści (cid:) są już widoczne w mailu.
        let is_inline = part.content_id().is_some() && mime.starts_with("image/");
        let name = part
            .attachment_name()
            .map(safe_name)
            .unwrap_or_else(|| format!("zalacznik-{}", index + 1));
        let contents = part.contents();
        let path = dir.join(format!("{index}-{name}"));
        std::fs::write(&path, contents)?;

        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        conn.execute(
            "INSERT INTO attachments (message_id, name, mime, size, path, is_inline)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                message_id,
                name,
                mime,
                contents.len() as i64,
                path.to_string_lossy(),
                is_inline as i64
            ],
        )?;
        result.push(Attachment {
            id: conn.last_insert_rowid(),
            name,
            mime,
            size: contents.len() as i64,
            path: path.to_string_lossy().to_string(),
            is_inline,
        });
    }
    Ok(result)
}

/// Kopiuje załącznik pod wskazaną ścieżkę (okno „Zapisz jako").
pub fn save_as(app: &AppHandle, attachment_id: i64, target: &str) -> Result<()> {
    let source: String = {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        conn.query_row("SELECT path FROM attachments WHERE id = ?1", [attachment_id], |r| {
            r.get(0)
        })?
    };
    std::fs::copy(source, target)?;
    Ok(())
}
