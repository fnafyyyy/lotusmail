//! Synchronizacja konfiguracji kont między urządzeniami.
//!
//! Zasada: **nasz serwer nie istnieje**. Paczka z kontami jest szyfrowana
//! hasłem, które zna wyłącznie użytkownik, a przenosi się ją albo wklejonym
//! kodem, albo wiadomością w jego własnej skrzynce IMAP. Serwer poczty widzi
//! nieczytelny blob - ani adresów, ani haseł.
//!
//! Klucz powstaje z hasła przez Argon2id, więc słabe hasło nie zamienia się
//! od razu w słaby klucz. Każda paczka ma własną sól i nonce.

use crate::db::Db;
use crate::error::{AppError, Result};
use base64::Engine;
use futures_util::TryStreamExt;
use chacha20poly1305::aead::{Aead, KeyInit, OsRng};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

/// Nagłówek paczki. Numer wersji jest w nim celowo: format może się zmienić,
/// a starsze urządzenie musi umieć powiedzieć „nie znam tej wersji" zamiast
/// odszyfrowywać śmieci.
const HEADER: &str = "LOTUSMAIL-SYNC-1";
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SyncAccount {
    pub email: String,
    pub display_name: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub auth_kind: String,
    pub login: String,
    pub sender_name: String,
    /// Hasło z pęku kluczy urządzenia źródłowego. W paczce jest jawne, ale
    /// sama paczka nigdy nie występuje w postaci niezaszyfrowanej poza pamięcią.
    pub password: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SyncPayload {
    pub version: u32,
    /// Znacznik czasu zapisu - przy konflikcie wygrywa nowsza paczka.
    pub updated_at: i64,
    /// Skąd przyszła. Wyłącznie do pokazania użytkownikowi, żeby wiedział,
    /// które urządzenie nadpisało konfigurację.
    pub device: String,
    pub accounts: Vec<SyncAccount>,
}

/// Wyprowadza klucz z hasła. Argon2id z domyślnymi parametrami crate'a -
/// świadomie nie zjeżdżamy niżej, bo to jedyna bariera między blobem
/// na cudzym serwerze a hasłami do poczty.
fn derive_key(passphrase: &str, salt: &[u8]) -> Result<Key> {
    use argon2::Argon2;
    let mut key = [0u8; 32];
    Argon2::default()
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|e| AppError::Other(format!("wyprowadzenie klucza: {e}")))?;
    Ok(Key::from(key))
}

/// Pakuje i szyfruje konfigurację. Wynik to tekst, który da się wkleić
/// w drugie urządzenie albo wysłać jako treść wiadomości IMAP.
pub fn seal(payload: &SyncPayload, passphrase: &str) -> Result<String> {
    if passphrase.trim().is_empty() {
        return Err(AppError::Other("hasło synchronizacji jest puste".into()));
    }
    use chacha20poly1305::aead::rand_core::RngCore;
    let mut salt = [0u8; SALT_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut salt);
    OsRng.fill_bytes(&mut nonce);

    let json = serde_json::to_vec(payload)
        .map_err(|e| AppError::Other(format!("serializacja paczki: {e}")))?;
    let cipher = XChaCha20Poly1305::new(&derive_key(passphrase, &salt)?);
    let ciphertext = cipher
        .encrypt(XNonce::from_slice(&nonce), json.as_ref())
        .map_err(|_| AppError::Other("szyfrowanie paczki nie powiodło się".into()))?;

    let mut blob = Vec::with_capacity(SALT_LEN + NONCE_LEN + ciphertext.len());
    blob.extend_from_slice(&salt);
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&ciphertext);
    Ok(format!(
        "{HEADER}\n{}",
        base64::engine::general_purpose::STANDARD.encode(&blob)
    ))
}

/// Odwrotność `seal`. Złe hasło objawia się błędem uwierzytelnienia AEAD,
/// więc nie da się po cichu wczytać przekłamanych danych.
pub fn open(blob: &str, passphrase: &str) -> Result<SyncPayload> {
    let body = blob
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>();
    let Some((head, rest)) = body.split_first() else {
        return Err(AppError::Other("pusty kod synchronizacji".into()));
    };
    if *head != HEADER {
        return Err(AppError::Other(format!(
            "nieznany format kodu (oczekiwano {HEADER})"
        )));
    }
    let raw = base64::engine::general_purpose::STANDARD
        .decode(rest.concat())
        .map_err(|_| AppError::Other("kod synchronizacji jest uszkodzony".into()))?;
    if raw.len() <= SALT_LEN + NONCE_LEN {
        return Err(AppError::Other("kod synchronizacji jest za krótki".into()));
    }
    let (salt, tail) = raw.split_at(SALT_LEN);
    let (nonce, ciphertext) = tail.split_at(NONCE_LEN);

    let cipher = XChaCha20Poly1305::new(&derive_key(passphrase, salt)?);
    let json = cipher
        .decrypt(XNonce::from_slice(nonce), ciphertext)
        .map_err(|_| AppError::Other("błędne hasło synchronizacji".into()))?;
    serde_json::from_slice(&json)
        .map_err(|e| AppError::Other(format!("paczka ma nieznaną budowę: {e}")))
}

/// Zbiera konfigurację z bazy i hasła z pęku kluczy tego urządzenia.
/// Konta demo pomijamy - nie ma ich po co przenosić.
pub fn collect(app: &AppHandle) -> Result<SyncPayload> {
    let rows: Vec<(String, String, String, u16, String, u16, String, String, String)> = {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT email, display_name, imap_host, imap_port, smtp_host, smtp_port,
                    auth_kind, login, sender_name
             FROM accounts WHERE auth_kind != 'demo' ORDER BY id",
        )?;
        let mapped = stmt.query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
                r.get(8)?,
            ))
        })?;
        mapped.collect::<rusqlite::Result<Vec<_>>>()?
    };

    let mut accounts = Vec::new();
    for (email, display_name, imap_host, imap_port, smtp_host, smtp_port, auth_kind, login, sender_name) in
        rows
    {
        // Konto bez hasła w pęku byłoby na drugim urządzeniu bezużyteczne -
        // lepiej je pominąć niż przenieść atrapę.
        let Ok(password) = crate::accounts::get_password(&email) else {
            eprintln!("[sync-config] pomijam {email}: brak hasła w pęku kluczy");
            continue;
        };
        accounts.push(SyncAccount {
            email,
            display_name,
            imap_host,
            imap_port,
            smtp_host,
            smtp_port,
            auth_kind,
            login,
            sender_name,
            password,
        });
    }

    Ok(SyncPayload {
        version: 1,
        updated_at: chrono::Utc::now().timestamp(),
        device: device_name(),
        accounts,
    })
}

/// Nazwa urządzenia do pokazania użytkownikowi. Bez ambicji unikalności -
/// ma tylko odpowiadać na pytanie „skąd przyszła ta zmiana".
fn device_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "nieznane urządzenie".into())
}

/// Ile kont dołożyła paczka i ile zaktualizowała.
#[derive(Debug, Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResult {
    pub added: usize,
    pub updated: usize,
    pub device: String,
}

/// Wgrywa paczkę do bazy tego urządzenia. Konta rozpoznajemy po adresie:
/// znane aktualizujemy, nieznane dokładamy. Nic nie usuwamy - kasowanie kont
/// przez synchronizację to zbyt duża władza jak na pierwszą wersję.
pub fn apply(app: &AppHandle, payload: &SyncPayload) -> Result<ApplyResult> {
    let mut out = ApplyResult {
        device: payload.device.clone(),
        ..Default::default()
    };
    for acc in &payload.accounts {
        // Hasło zapisujemy najpierw: konto bez hasła w pęku nie zsynchronizuje
        // się i tylko wygląda na dodane.
        crate::accounts::store_password(&acc.email, &acc.password)?;

        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        let existing: Option<i64> = conn
            .query_row(
                "SELECT id FROM accounts WHERE email = ?1",
                [&acc.email],
                |r| r.get(0),
            )
            .ok();
        match existing {
            Some(id) => {
                conn.execute(
                    "UPDATE accounts SET display_name = ?2, imap_host = ?3, imap_port = ?4,
                            smtp_host = ?5, smtp_port = ?6, auth_kind = ?7, login = ?8,
                            sender_name = ?9
                     WHERE id = ?1",
                    rusqlite::params![
                        id,
                        acc.display_name,
                        acc.imap_host,
                        acc.imap_port,
                        acc.smtp_host,
                        acc.smtp_port,
                        acc.auth_kind,
                        acc.login,
                        acc.sender_name
                    ],
                )?;
                out.updated += 1;
            }
            None => {
                conn.execute(
                    "INSERT INTO accounts (email, display_name, imap_host, imap_port,
                                           smtp_host, smtp_port, auth_kind, login, sender_name)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    rusqlite::params![
                        acc.email,
                        acc.display_name,
                        acc.imap_host,
                        acc.imap_port,
                        acc.smtp_host,
                        acc.smtp_port,
                        acc.auth_kind,
                        acc.login,
                        acc.sender_name
                    ],
                )?;
                out.added += 1;
            }
        }
    }
    Ok(out)
}


/// Folder, w którym trzymamy paczkę na serwerze użytkownika.
const CARRIER_FOLDER: &str = "LotusMail";
const CARRIER_SUBJECT: &str = "LotusMail sync v1";

/// Dane dostępowe konta-nośnika.
struct Carrier {
    email: String,
    login: String,
    host: String,
    port: u16,
}

fn carrier_of(app: &AppHandle, account_id: i64) -> Result<Carrier> {
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
    let login = if login.is_empty() { email.clone() } else { login };
    Ok(Carrier {
        email,
        login,
        host,
        port,
    })
}

/// Składa wiadomość niosącą paczkę. Zwykły `text/plain` - żaden klient poczty
/// nie musi tego rozumieć, ma tylko przechować.
fn carrier_message(email: &str, blob: &str) -> Vec<u8> {
    let date = chrono::Utc::now().format("%a, %d %b %Y %H:%M:%S +0000");
    let body = blob.replace('\n', "\r\n");
    format!(
        "From: LotusMail <{email}>\r\n\
         To: LotusMail <{email}>\r\n\
         Subject: {CARRIER_SUBJECT}\r\n\
         Date: {date}\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\
         \r\n\
         {body}\r\n"
    )
    .into_bytes()
}

/// Odkłada zaszyfrowaną konfigurację w skrzynce użytkownika.
///
/// Stare paczki kasujemy przed dołożeniem nowej - w folderze ma leżeć dokładnie
/// jedna, inaczej po miesiącu byłoby ich sto i nie wiadomo, która obowiązuje.
pub async fn push(app: &AppHandle, account_id: i64, passphrase: &str) -> Result<usize> {
    let payload = collect(app)?;
    let count = payload.accounts.len();
    let blob = seal(&payload, passphrase)?;
    let carrier = carrier_of(app, account_id)?;
    let password = crate::accounts::get_password(&carrier.email)?;

    let mut session =
        crate::sync::connect_session(&carrier.host, carrier.port, &carrier.login, &password).await?;
    // Serwer, który już ma ten folder, odpowie błędem - i słusznie, więc go
    // pomijamy zamiast traktować jako awarię.
    let _ = session.create(CARRIER_FOLDER).await;

    if session.select(CARRIER_FOLDER).await.is_ok() {
        let old: Vec<u32> = session.uid_search("ALL").await?.into_iter().collect();
        if !old.is_empty() {
            let uid_set = old
                .iter()
                .map(|u| u.to_string())
                .collect::<Vec<_>>()
                .join(",");
            let stream = session.uid_store(&uid_set, "+FLAGS.SILENT (\\Deleted)").await?;
            stream.try_collect::<Vec<_>>().await?;
            // Rozbite na dwa kroki, bo tymczasowy wynik `if let` trzymałby
            // pożyczkę sesji przez całą gałąź `else`.
            let mut expunged = false;
            if let Ok(stream) = session.uid_expunge(&uid_set).await {
                stream.try_collect::<Vec<_>>().await.ok();
                expunged = true;
            }
            if !expunged {
                if let Ok(stream) = session.expunge().await {
                    stream.try_collect::<Vec<_>>().await.ok();
                }
            }
        }
    }

    let raw = carrier_message(&carrier.email, &blob);
    session
        .append(CARRIER_FOLDER, Some("(\\Seen)"), None, &raw)
        .await
        .map_err(|e| AppError::Other(format!("zapis paczki w „{CARRIER_FOLDER}”: {e}")))?;
    session.logout().await.ok();
    Ok(count)
}

/// Pobiera paczkę ze skrzynki i wgrywa ją do tego urządzenia.
pub async fn pull(app: &AppHandle, account_id: i64, passphrase: &str) -> Result<ApplyResult> {
    let carrier = carrier_of(app, account_id)?;
    let password = crate::accounts::get_password(&carrier.email)?;

    let mut session =
        crate::sync::connect_session(&carrier.host, carrier.port, &carrier.login, &password).await?;
    if session.select(CARRIER_FOLDER).await.is_err() {
        session.logout().await.ok();
        return Err(AppError::Other(format!(
            "na tym koncie nie ma jeszcze folderu „{CARRIER_FOLDER}” - wyślij konfigurację z drugiego urządzenia"
        )));
    }
    let uids: Vec<u32> = session.uid_search("ALL").await?.into_iter().collect();
    let Some(newest) = uids.iter().copied().max() else {
        session.logout().await.ok();
        return Err(AppError::Other("folder synchronizacji jest pusty".into()));
    };

    // BODY.PEEK, żeby nie oznaczać paczki jako przeczytanej - to nie jest
    // wiadomość dla człowieka.
    let mut text = String::new();
    {
        let mut stream = session
            .uid_fetch(newest.to_string(), "BODY.PEEK[TEXT]")
            .await?;
        while let Some(item) = stream.try_next().await? {
            if let Some(body) = item.text() {
                text = String::from_utf8_lossy(body).to_string();
                break;
            }
        }
    }
    session.logout().await.ok();

    if text.trim().is_empty() {
        return Err(AppError::Other("paczka w skrzynce jest pusta".into()));
    }
    let payload = open(&text, passphrase)?;
    apply(app, &payload)
}

#[cfg(test)]
mod testy {
    use super::*;

    fn paczka() -> SyncPayload {
        SyncPayload {
            version: 1,
            updated_at: 1_700_000_000,
            device: "TEST".into(),
            accounts: vec![SyncAccount {
                email: "a@example.com".into(),
                display_name: "A".into(),
                imap_host: "imap.example.com".into(),
                imap_port: 993,
                smtp_host: "smtp.example.com".into(),
                smtp_port: 465,
                auth_kind: "password".into(),
                login: String::new(),
                sender_name: "Adrian".into(),
                password: "tajne-hasło-ąęć".into(),
            }],
        }
    }

    #[test]
    fn paczka_wraca_w_calosci() {
        let blob = seal(&paczka(), "moje hasło").expect("szyfrowanie");
        let back = open(&blob, "moje hasło").expect("odszyfrowanie");
        assert_eq!(back.accounts.len(), 1);
        assert_eq!(back.accounts[0].password, "tajne-hasło-ąęć");
        assert_eq!(back.device, "TEST");
    }

    #[test]
    fn zle_haslo_nie_przechodzi() {
        let blob = seal(&paczka(), "moje hasło").expect("szyfrowanie");
        assert!(open(&blob, "inne hasło").is_err());
    }

    #[test]
    fn dwie_paczki_roznia_sie_mimo_tych_samych_danych() {
        // Sól i nonce są losowe, więc identyczna konfiguracja nie daje
        // identycznego blobu - inaczej podsłuchujący widziałby, że nic
        // się nie zmieniło.
        let a = seal(&paczka(), "h").expect("a");
        let b = seal(&paczka(), "h").expect("b");
        assert_ne!(a, b);
    }
}
