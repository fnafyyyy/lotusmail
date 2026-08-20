//! Wysyłka SMTP (lettre) - przetwarzanie kolejki `outbox`.
//!
//! Wiadomości trafiają do kolejki komendą `queue_send` (od razu albo
//! z `send_at` w przyszłości - „wyślij później"), a ten moduł wysyła je
//! przez SMTP konta i oznacza status sent/failed.

use crate::auth::{self, Secret};
use crate::db::Db;
use crate::error::{AppError, Result};
use crate::sync;
use lettre::message::header::{ContentDisposition, ContentType};
use lettre::message::{Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::{Credentials, Mechanism};
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use rusqlite::OptionalExtension;
use tauri::{AppHandle, Emitter, Manager};

struct QueuedMail {
    id: i64,
    account_id: i64,
    to_addrs: String,
    cc_addrs: String,
    bcc_addrs: String,
    subject: String,
    body_text: String,
    body_html: Option<String>,
    /// Message-ID wiadomości, na którą odpowiadamy (bez nawiasów kątowych).
    in_reply_to: Option<String>,
    references: Option<String>,
}

/// Wysyła wszystkie zaległe wiadomości z kolejki. Wywoływane po `queue_send`
/// i cyklicznie przez harmonogram (dla „wyślij później").
pub async fn process_outbox(app: &AppHandle) {
    let due: Vec<QueuedMail> = {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        let now = chrono::Utc::now().timestamp();
        let Ok(mut stmt) = conn.prepare(
            "SELECT id, account_id, to_addrs, COALESCE(cc_addrs, ''), COALESCE(bcc_addrs, ''),
                    subject, COALESCE(body_text, ''), body_html, in_reply_to, msg_references
             FROM outbox
             WHERE status = 'queued' AND (send_at IS NULL OR send_at <= ?1)",
        ) else {
            return;
        };
        stmt.query_map([now], |r| {
            Ok(QueuedMail {
                id: r.get(0)?,
                account_id: r.get(1)?,
                to_addrs: r.get(2)?,
                cc_addrs: r.get(3)?,
                bcc_addrs: r.get(4)?,
                subject: r.get(5)?,
                body_text: r.get(6)?,
                body_html: r.get(7)?,
                in_reply_to: r.get(8)?,
                references: r.get(9)?,
            })
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    };
    if due.is_empty() {
        return;
    }

    let mut sent_any = false;
    for mail in due {
        set_status(app, mail.id, "sending", None);
        let _ = app.emit("sync-status", format!("Wysyłam: {}", mail.subject));
        match send_one(app, &mail).await {
            Ok(raw) => {
                set_status(app, mail.id, "sent", None);
                sent_any = true;
                let _ = app.emit("outbox-sent", mail.subject.clone());
                // Kopia w „Wysłanych" to dodatek, a nie warunek powodzenia:
                // mail już poszedł do adresata, więc potknięcie IMAP-a nie
                // może go oznaczyć jako niewysłanego.
                if let Err(e) = append_to_sent(app, mail.account_id, &raw).await {
                    eprintln!("[send] kopia w Wysłanych (outbox {}): {e}", mail.id);
                    let _ = app.emit(
                        "sync-error",
                        format!(
                            "wysłano „{}”, ale kopia w Wysłanych się nie zapisała: {e}",
                            mail.subject
                        ),
                    );
                }
            }
            Err(e) => {
                eprintln!("[send] outbox {}: {e}", mail.id);
                set_status(app, mail.id, "failed", Some(&e.to_string()));
                let _ = app.emit("sync-error", format!("wysyłka „{}”: {e}", mail.subject));
            }
        }
        let _ = app.emit("sync-status", "");
    }
    if sent_any {
        // Tryb szybki, nie pełny: interesują nas trzy foldery - Wysłane
        // (kopia właśnie odłożona przez APPEND), Odebrane i Kosz. Pełny
        // przebieg chodzi po wszystkich folderach i dociąga zaległą historię,
        // więc na dużej skrzynce trwa minutami - a przez ten czas wysłany
        // mail nie pokazywał się w Wysłanych, mimo że leżał już na serwerze.
        sync::sync_all_mode(app, sync::SyncMode::Quick).await;
    }
}

/// Załączniki wiadomości z kolejki: (nazwa, typ MIME, bajty).
fn load_attachments(app: &AppHandle, outbox_id: i64) -> Vec<(String, String, Vec<u8>)> {
    let db = app.state::<Db>();
    let conn = db.0.lock().unwrap();
    let Ok(mut stmt) = conn.prepare(
        "SELECT filename, mime, data FROM outbox_attachments WHERE outbox_id = ?1 ORDER BY id",
    ) else {
        return Vec::new();
    };
    stmt.query_map([outbox_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
}

fn set_status(app: &AppHandle, id: i64, status: &str, error: Option<&str>) {
    let db = app.state::<Db>();
    let conn = db.0.lock().unwrap();
    let _ = conn.execute(
        "UPDATE outbox SET status = ?2, error = ?3 WHERE id = ?1",
        rusqlite::params![id, status, error],
    );
}

/// Odkłada kopię wysłanego maila w serwerowym folderze „Wysłane". SMTP sam
/// jej tam nie zostawia - bez tego wiadomość widać wyłącznie u odbiorcy,
/// a w LotusMailu i każdym innym kliencie folder wysłanych zostaje pusty.
///
/// Gmail jest wyjątkiem: kopię zapisuje sam, gdy wysyłasz przez jego SMTP,
/// więc APPEND zrobiłby drugi egzemplarz tej samej wiadomości.
async fn append_to_sent(app: &AppHandle, account_id: i64, raw: &[u8]) -> Result<()> {
    let (email, login, host, port, folder) = {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        let (email, login, host, port) = conn.query_row(
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
        )?;
        // Nazwa folderu leży w bazie w surowej postaci (zmodyfikowany UTF-7) -
        // dokładnie takiej, jakiej oczekuje APPEND.
        let folder: Option<String> = conn
            .query_row(
                "SELECT name FROM folders WHERE account_id = ?1 AND kind = 'sent' LIMIT 1",
                [account_id],
                |r| r.get(0),
            )
            .optional()?;
        (email, login, host, port, folder)
    };

    if host.is_empty() {
        return Err(AppError::Other(
            "konto nie ma skonfigurowanego serwera IMAP".into(),
        ));
    }
    if host.to_lowercase().contains("gmail.com") {
        return Ok(());
    }
    let Some(folder) = folder else {
        return Err(AppError::Other(
            "nie rozpoznano folderu Wysłane na tym koncie".into(),
        ));
    };

    let login = if login.is_empty() { email.clone() } else { login };
    let secret = auth::secret_for(app, &email).await?;
    let mut session = sync::connect_with(&host, port, &login, &secret).await?;
    // Własny mail jest z definicji przeczytany - bez `\Seen` folder wysłanych
    // pokazywałby licznik nieprzeczytanych.
    let appended = session
        .append(&folder, Some("(\\Seen)"), None, raw)
        .await
        .map_err(|e| AppError::Other(format!("APPEND do „{folder}”: {e}")));
    session.logout().await.ok();
    appended
}

/// Zwraca surową postać wysłanej wiadomości - tę samą, którą odkładamy
/// w folderze „Wysłane".
async fn send_one(app: &AppHandle, mail: &QueuedMail) -> Result<Vec<u8>> {
    let (email, sender_name, login, host, port) = {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        conn.query_row(
            "SELECT email, sender_name, login, smtp_host, smtp_port FROM accounts WHERE id = ?1",
            [mail.account_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, u16>(4)?,
                ))
            },
        )?
    };
    if host.is_empty() {
        return Err(AppError::Other(
            "konto nie ma skonfigurowanego serwera SMTP".into(),
        ));
    }
    let login = if login.is_empty() { email.clone() } else { login };
    let secret = auth::secret_for(app, &email).await?;

    let from_addr = email
        .parse()
        .map_err(|e| AppError::Other(format!("zły adres nadawcy „{email}”: {e}")))?;
    let mut builder = Message::builder()
        .from(Mailbox::new(
            Some(sender_name).filter(|s| !s.trim().is_empty()),
            from_addr,
        ))
        .subject(&mail.subject);
    for addr in mail.to_addrs.split(',') {
        let addr = addr.trim();
        if addr.is_empty() {
            continue;
        }
        builder = builder.to(addr
            .parse::<Mailbox>()
            .map_err(|e| AppError::Other(format!("zły adres odbiorcy „{addr}”: {e}")))?);
    }
    for addr in mail.cc_addrs.split(',') {
        let addr = addr.trim();
        if addr.is_empty() {
            continue;
        }
        builder = builder.cc(addr
            .parse::<Mailbox>()
            .map_err(|e| AppError::Other(format!("zły adres kopii „{addr}”: {e}")))?);
    }
    // UDW nie pojawia się w nagłówkach - lettre dokłada tych odbiorców
    // wyłącznie do koperty SMTP, więc pozostali ich nie zobaczą.
    for addr in mail.bcc_addrs.split(',') {
        let addr = addr.trim();
        if addr.is_empty() {
            continue;
        }
        builder = builder.bcc(addr
            .parse::<Mailbox>()
            .map_err(|e| AppError::Other(format!("zły adres ukrytej kopii „{addr}”: {e}")))?);
    }

    // Wątkowanie: bez tych nagłówków odpowiedź wisi u odbiorcy jako osobny
    // mail. Baza trzyma identyfikatory bez nawiasów kątowych, a RFC 5322
    // ich wymaga - stąd `angled`.
    let angled = |id: &str| -> String {
        id.split_whitespace()
            .map(|part| {
                if part.starts_with('<') {
                    part.to_string()
                } else {
                    format!("<{part}>")
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    };
    if let Some(id) = mail.in_reply_to.as_deref().filter(|s| !s.trim().is_empty()) {
        builder = builder.in_reply_to(angled(id));
    }
    if let Some(refs) = mail.references.as_deref().filter(|s| !s.trim().is_empty()) {
        builder = builder.references(angled(refs));
    }

    let html = mail
        .body_html
        .as_ref()
        .filter(|h| !h.trim().is_empty())
        .cloned();
    let attachments = load_attachments(app, mail.id);

    // Struktura wiadomości zależy od tego, co w niej jest: sam tekst, tekst
    // z HTML (multipart/alternative), a przy plikach całość opakowana
    // w multipart/mixed - inaczej klienty pokazują załączniki zamiast treści.
    let message = match (html, attachments.is_empty()) {
        (None, true) => builder.body(mail.body_text.clone()),
        (Some(html), true) => builder.multipart(MultiPart::alternative_plain_html(
            mail.body_text.clone(),
            html,
        )),
        (html, false) => {
            let mut mixed = match html {
                Some(html) => MultiPart::mixed().multipart(MultiPart::alternative_plain_html(
                    mail.body_text.clone(),
                    html,
                )),
                None => MultiPart::mixed().singlepart(
                    SinglePart::builder()
                        .header(ContentType::TEXT_PLAIN)
                        .body(mail.body_text.clone()),
                ),
            };
            for (filename, mime, data) in attachments {
                let content_type = ContentType::parse(&mime)
                    .unwrap_or(ContentType::parse("application/octet-stream").unwrap());
                mixed = mixed.singlepart(
                    SinglePart::builder()
                        .header(content_type)
                        .header(ContentDisposition::attachment(&filename))
                        .body(data),
                );
            }
            builder.multipart(mixed)
        }
    }
    .map_err(|e| AppError::Other(format!("budowanie wiadomości: {e}")))?;

    // Port 465 = SMTPS (TLS od początku), inne (587/25) = STARTTLS.
    let transport = if port == 465 {
        AsyncSmtpTransport::<Tokio1Executor>::relay(&host)
    } else {
        AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&host)
    }
    .map_err(|e| AppError::Other(format!("połączenie SMTP z {host}: {e}")))?
    .port(port);
    // lettre builds the XOAUTH2 string itself from the user and the "password",
    // so an access token goes in the same slot - only the mechanism differs.
    // It has to be named explicitly, otherwise lettre negotiates PLAIN and the
    // server rejects the token as a malformed password.
    let transport = match &secret {
        Secret::Password(password) => {
            transport.credentials(Credentials::new(login, password.clone()))
        }
        Secret::OAuth(token) => transport
            .credentials(Credentials::new(login, token.clone()))
            .authentication(vec![Mechanism::Xoauth2]),
    }
    .build();

    // Surowa postać musi powstać przed wysyłką - `send` przejmuje wiadomość.
    let raw = message.formatted();
    transport
        .send(message)
        .await
        .map_err(|e| AppError::Other(format!("serwer SMTP odrzucił wysyłkę: {e}")))?;
    Ok(raw)
}
