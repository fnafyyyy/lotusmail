//! Silnik synchronizacji IMAP i harmonogram zadań w tle.
//!
//! Architektura offline-first: sieć <-> ten moduł <-> SQLite <-> UI.
//! Interfejs czyta wyłącznie z lokalnej bazy i nigdy nie czeka na sieć.
//! Uwaga na blokady: mutex bazy nigdy nie jest trzymany przez `await` -
//! dane zbieramy z sieci do wektorów, a zapisujemy w krótkich sekcjach.

use crate::db::Db;
use crate::error::{AppError, Result};
use crate::{accounts, mail};
use async_imap::types::{Fetch, Flag, Name, NameAttribute};
use futures_util::TryStreamExt;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

type ImapSession = async_imap::Session<tokio_native_tls::TlsStream<tokio::net::TcpStream>>;

/// Ile wiadomości pobieramy z folderu przy pierwszym kontakcie...
const FIRST_SYNC_LIMIT: usize = 500;
/// ...a ile dokładamy przy każdej kolejnej synchronizacji, aż pobierzemy całość.
const BACKFILL_BATCH: usize = 250;
/// Wielkość jednej paczki pobieranej z serwera (za duża = długie czekanie).
const FETCH_CHUNK: usize = 25;
/// Ile ostatnich wiadomości folderu sprawdzamy pod kątem flag z serwera.
const FLAG_WINDOW: i64 = 400;
/// Ile najdłużej może trwać synchronizacja jednego konta.
///
/// To bezpiecznik na martwe gniazda, a nie limit na pracę. Po uśpieniu laptopa
/// odczyt z takiego gniazda potrafi wisieć kwadransami - system czeka na
/// retransmisje, których nikt nie odbierze. Harmonogram jest jedną pętlą, więc
/// bez limitu jeden zawieszony odczyt zatrzymywał wszystko: drzemki, kolejkę
/// wysyłki i pozostałe konta, a pasek stanu zostawał na „Pobieram…".
///
/// Budżety są różne, bo tryby robią co innego. Szybki zagląda tylko do trzech
/// folderów i po dwóch minutach na pewno coś jest nie tak. Pełny przechodzi
/// wszystkie foldery, pobiera flagi i dociąga zaległą historię - na dużym
/// Gmailu potrafi zejść kilkanaście minut i to jest normalne.
const QUICK_TIMEOUT: Duration = Duration::from_secs(120);
const FULL_TIMEOUT: Duration = Duration::from_secs(900);

/// Łączy się z serwerem IMAP po TLS i loguje. Wspólne dla testu i synchronizacji.
pub async fn connect_session(host: &str, port: u16, login: &str, password: &str) -> Result<ImapSession> {
    if port == 143 {
        return Err(AppError::Other(
            "serwer oferuje tylko STARTTLS (port 143) - na razie obsługiwany jest bezpośredni TLS (zwykle port 993)".into(),
        ));
    }
    let tcp = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::net::TcpStream::connect((host, port)),
    )
    .await
    .map_err(|_| AppError::Other(format!("przekroczono czas łączenia z {host}:{port}")))??;
    let tls = tokio_native_tls::TlsConnector::from(native_tls::TlsConnector::new()?);
    let stream = tls
        .connect(host, tcp)
        .await
        .map_err(|e| AppError::Other(format!("uścisk dłoni TLS z {host} nie powiódł się: {e}")))?;
    let client = async_imap::Client::new(stream);
    client
        .login(login, password)
        .await
        .map_err(|(e, _)| AppError::Other(format!("serwer odrzucił logowanie: {e}")))
}

/// Sprawdza poświadczenia bez zapisywania czegokolwiek.
pub async fn try_login(host: &str, port: u16, login: &str, password: &str) -> Result<()> {
    let mut session = connect_session(host, port, login, password).await?;
    session.logout().await.ok();
    Ok(())
}

/// Foldery-etykiety Gmaila („Wszystkie", „Ważne", „Oznaczone gwiazdką")
/// zawierają te same wiadomości co reszta skrzynki - pobieranie ich znaczyłoby
/// tyle samo maili trzy razy.
fn is_virtual_folder(name: &Name) -> bool {
    let attrs = format!("{:?}", name.attributes()).to_lowercase();
    if attrs.contains("all") || attrs.contains("important") || attrs.contains("flagged") {
        return true;
    }
    let decoded = utf7_imap::decode_utf7_imap(name.name().to_string()).to_lowercase();
    matches!(
        decoded.as_str(),
        "[gmail]/wszystkie"
            | "[gmail]/all mail"
            | "[gmail]/ważne"
            | "[gmail]/important"
            | "[gmail]/oznaczone gwiazdką"
            | "[gmail]/starred"
    )
}

fn folder_kind(name: &str, attrs: &[NameAttribute]) -> &'static str {
    let attr_str = format!("{attrs:?}").to_lowercase();
    let n = name.to_lowercase();
    if n == "inbox" {
        "inbox"
    } else if attr_str.contains("sent") || n.contains("sent") || n.contains("wysłane") || n.contains("wyslane") {
        "sent"
    } else if attr_str.contains("draft") || n.contains("draft") || n.contains("robocze") {
        "drafts"
    } else if attr_str.contains("trash") || n.contains("trash") || n.contains("deleted") || n.contains("kosz") {
        "trash"
    } else if attr_str.contains("junk") || n.contains("junk") || n.contains("spam") {
        "spam"
    } else if attr_str.contains("archive") || n.contains("archi") {
        "archive"
    } else {
        "custom"
    }
}

/// Nazwa folderu do pokazania. Tłumaczymy tylko standardowe angielskie nazwy;
/// gdy przetłumaczona nazwa jest już zajęta (serwer ma np. i „Trash",
/// i „Deleted Items"), zostawiamy oryginalną - inaczej powstają duplikaty
/// nie do rozróżnienia.
fn display_name(
    raw_decoded: &str,
    delimiter: &str,
    used: &mut std::collections::HashSet<String>,
) -> String {
    let leaf = raw_decoded
        .rsplit(delimiter)
        .next()
        .unwrap_or(raw_decoded)
        .to_string();
    let translated = match leaf.to_lowercase().as_str() {
        "inbox" => Some("Odebrane"),
        "sent" | "sent items" | "sent messages" => Some("Wysłane"),
        "drafts" => Some("Kopie robocze"),
        "trash" | "deleted items" => Some("Kosz"),
        "junk" | "junk e-mail" | "spam" => Some("Spam"),
        "archive" => Some("Archiwum"),
        _ => None,
    };
    let mut name = match translated {
        Some(t) if !used.contains(t) => t.to_string(),
        _ => leaf.clone(),
    };
    // Gdy i oryginał się powtarza, dokładamy ścieżkę nadrzędną.
    if used.contains(&name) {
        name = raw_decoded.replace(delimiter, " / ");
    }
    used.insert(name.clone());
    name
}

struct FetchedMessage {
    uid: u32,
    seen: bool,
    flagged: bool,
    internal_date: i64,
    raw: Vec<u8>,
}

fn to_fetched(f: &Fetch) -> Option<FetchedMessage> {
    Some(FetchedMessage {
        uid: f.uid?,
        seen: f.flags().any(|fl| matches!(fl, Flag::Seen)),
        flagged: f.flags().any(|fl| matches!(fl, Flag::Flagged)),
        internal_date: f.internal_date().map(|d| d.timestamp()).unwrap_or(0),
        raw: f.body()?.to_vec(),
    })
}

/// Tryb synchronizacji: szybki zagląda tylko do skrzynek odbiorczych po nową
/// pocztę, pełny przerabia wszystkie foldery i dociąga zaległą historię.
#[derive(Clone, Copy, PartialEq)]
pub enum SyncMode {
    Quick,
    Full,
}

pub async fn sync_account(app: &AppHandle, account_id: i64) -> Result<()> {
    sync_account_mode(app, account_id, SyncMode::Full).await
}

pub async fn sync_account_mode(
    app: &AppHandle,
    account_id: i64,
    mode: SyncMode,
) -> Result<()> {
    // Krótka sekcja z blokadą: dane konta.
    let (email, login, host, port, auth_kind) = {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        conn.query_row(
            "SELECT email, login, imap_host, imap_port, auth_kind FROM accounts WHERE id = ?1",
            [account_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, u16>(3)?,
                    r.get::<_, String>(4)?,
                ))
            },
        )?
    };
    if auth_kind == "demo" {
        return Ok(());
    }
    let login = if login.is_empty() { email.clone() } else { login };
    let password = accounts::get_password(&email)?;

    let mut session = connect_session(&host, port, &login, &password).await?;

    // Lista folderów z serwera.
    let names: Vec<Name> = session.list(Some(""), Some("*")).await?.try_collect().await?;
    // Nazwy folderów IMAP przychodzą w zmodyfikowanym UTF-7 (np. "Wys&AUI-ane")
    // - do rozpoznania rodzaju i wyświetlania używamy postaci zdekodowanej,
    // a surowa nazwa zostaje kluczem i argumentem SELECT.
    // Znak rozdzielający poziomy folderów: Gmail używa "/", Dovecot "." itd.
    let delimiter = names
        .iter()
        .find_map(|n| n.delimiter())
        .unwrap_or("/")
        .to_string();
    let selectable: Vec<(String, String, &'static str)> = names
        .iter()
        .filter(|n| !n.attributes().iter().any(|a| matches!(a, NameAttribute::NoSelect)))
        .filter(|n| !is_virtual_folder(n))
        .map(|n| {
            let decoded = utf7_imap::decode_utf7_imap(n.name().to_string());
            let kind = folder_kind(&decoded, n.attributes());
            (n.name().to_string(), decoded, kind)
        })
        .collect();

    // Upsert folderów i odczyt ich stanu lokalnego.
    let folders: Vec<(i64, String, Option<i64>, i64, bool, i64)> = {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        // Nazwy wyliczamy w kolejności: INBOX pierwszy, potem alfabetycznie,
        // żeby standardowe foldery dostały ładne nazwy przed nietypowymi.
        let mut used = std::collections::HashSet::new();
        let mut ordered: Vec<&(String, String, &str)> = selectable.iter().collect();
        ordered.sort_by_key(|(_, decoded, kind)| {
            (*kind != "inbox", decoded.to_lowercase())
        });
        for (name, decoded, kind) in ordered {
            conn.execute(
                "INSERT INTO folders (account_id, name, display_name, kind) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(account_id, name) DO UPDATE SET kind = excluded.kind, display_name = excluded.display_name",
                rusqlite::params![
                    account_id,
                    name,
                    display_name(decoded, &delimiter, &mut used),
                    kind
                ],
            )?;
        }
        // Foldery skasowane po stronie serwera (także w innym programie) oraz
        // pomijane etykiety Gmaila znikają też u nas - inaczej zostawałyby
        // na zawsze w panelu bocznym razem ze zdublowanymi wiadomościami.
        let on_server: std::collections::HashSet<&str> = names
            .iter()
            .filter(|n| !is_virtual_folder(n))
            .map(|n| n.name())
            .collect();
        let local: Vec<(i64, String)> = {
            let mut stmt =
                conn.prepare("SELECT id, name FROM folders WHERE account_id = ?1")?;
            let rows = stmt.query_map([account_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (id, name) in local {
            if !on_server.contains(name.as_str()) {
                conn.execute("DELETE FROM folders WHERE id = ?1", [id])?;
            }
        }
        // Skrzynka odbiorcza zawsze pierwsza - nowa poczta nie może czekać,
        // aż przerobimy archiwum. W trybie szybkim doglądamy jeszcze Kosza
        // i Wysłanych, żeby efekty usuwania i wysyłki było widać od razu.
        let mut stmt = conn.prepare(
            "SELECT f.id, f.name, f.uid_validity, f.last_seen_uid, f.backfilled,
                    (SELECT COUNT(*) FROM messages m WHERE m.folder_id = f.id AND m.uid > 0)
             FROM folders f
             WHERE f.account_id = ?1
               AND (?2 = 0 OR f.kind IN ('inbox', 'trash', 'sent'))
             ORDER BY f.kind = 'inbox' DESC, f.id",
        )?;
        let quick = (mode == SyncMode::Quick) as i64;
        let rows = stmt
            .query_map(rusqlite::params![account_id, quick], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get::<_, i64>(4)? != 0, r.get(5)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };

    // Foldery, do których reguły przeniosły coś w tym przebiegu - doczytamy je
    // na końcu, po zakończeniu obchodu.
    let mut rule_targets: std::collections::HashSet<(i64, String)> = std::collections::HashSet::new();

    for (folder_id, folder_name, stored_uidv, last_seen, backfilled, local_count) in folders {
        let Some((_, decoded, _)) = selectable.iter().find(|(n, ..)| *n == folder_name) else {
            continue;
        };
        let Ok(mailbox) = session.select(&folder_name).await else { continue };
        // Najpierw wypychamy lokalne zmiany statusów, żeby serwerowe flagi
        // pobrane niżej ich nie nadpisały.
        if let Err(e) = push_dirty_flags(app, &mut session, folder_id).await {
            eprintln!("[sync] wysyłka flag folderu {folder_id}: {e}");
        }
        let uidv = i64::from(mailbox.uid_validity.unwrap_or(0));
        let exists = mailbox.exists;

        // Zmiana UIDVALIDITY unieważnia wszystkie lokalne UID-y tego folderu.
        let mut last_seen = last_seen;
        if stored_uidv != Some(uidv) {
            let db = app.state::<Db>();
            let conn = db.0.lock().unwrap();
            conn.execute("DELETE FROM messages WHERE folder_id = ?1", [folder_id])?;
            conn.execute(
                "UPDATE folders SET uid_validity = ?2, last_seen_uid = 0 WHERE id = ?1",
                rusqlite::params![folder_id, uidv],
            )?;
            last_seen = 0;
        }
        if exists == 0 {
            continue;
        }

        // Czego brakuje lokalnie. Drogie porównanie pełnej listy UID-ów robimy
        // tylko wtedy, gdy jest po co: w trybie szybkim nigdy, a w pełnym -
        // dopóki folder nie jest kompletny albo dopóki liczba wiadomości na
        // serwerze zgadza się z lokalną. Folder domknięty i zgodny sprawdzamy
        // tak samo tanio jak w trybie szybkim: jednym pytaniem o nowe UID-y.
        let settled = backfilled && local_count == i64::from(exists);
        let cheap = mode == SyncMode::Quick || settled;
        let mut remaining = 0usize;
        let mut missing: Vec<u32> = if cheap {
            let found = session.uid_search(format!("UID {}:*", last_seen + 1)).await?;
            let known: std::collections::HashSet<u32> = {
                let db = app.state::<Db>();
                let conn = db.0.lock().unwrap();
                let mut stmt = conn.prepare(
                    "SELECT uid FROM messages WHERE folder_id = ?1 AND uid > ?2",
                )?;
                let rows = stmt.query_map(rusqlite::params![folder_id, last_seen], |r| r.get(0))?;
                rows.collect::<rusqlite::Result<_>>()?
            };
            // `n:*` zwraca też ostatnią wiadomość, gdy n przekracza najwyższy UID.
            let mut fresh: Vec<u32> = found
                .into_iter()
                .filter(|u| i64::from(*u) > last_seen && !known.contains(u))
                .collect();
            // Bezpiecznik: gdyby licznik UID-ów kiedykolwiek się rozjechał,
            // tania ścieżka zwróciłaby cały folder i jeden przebieg ciągnąłby
            // tysiące wiadomości, przekraczając limit czasu. Bierzemy najnowsze
            // i tyle; reszta dojdzie kolejnymi przebiegami.
            if fresh.len() > BACKFILL_BATCH {
                fresh.sort_unstable_by(|a, b| b.cmp(a));
                remaining = fresh.len() - BACKFILL_BATCH;
                fresh.truncate(BACKFILL_BATCH);
            }
            fresh
        } else {
            let server_uids = session.uid_search("ALL").await?;
            let local_uids: std::collections::HashSet<u32> = {
                let db = app.state::<Db>();
                let conn = db.0.lock().unwrap();
                let mut stmt =
                    conn.prepare("SELECT uid FROM messages WHERE folder_id = ?1 AND uid > 0")?;
                let rows = stmt.query_map([folder_id], |r| r.get::<_, u32>(0))?;
                rows.collect::<rusqlite::Result<std::collections::HashSet<_>>>()?
            };
            // Wiadomości skasowane poza LotusMailem znikają też u nas - inaczej
            // lokalna liczba nigdy nie zgodzi się z serwerem i folder w kółko
            // trafiałby na drogą ścieżkę.
            let vanished: Vec<u32> = local_uids.difference(&server_uids).copied().collect();
            if !vanished.is_empty() {
                let db = app.state::<Db>();
                let conn = db.0.lock().unwrap();
                for uid in &vanished {
                    conn.execute(
                        "DELETE FROM messages WHERE folder_id = ?1 AND uid = ?2",
                        rusqlite::params![folder_id, uid],
                    )?;
                }
            }

            let budget = if local_uids.is_empty() { FIRST_SYNC_LIMIT } else { BACKFILL_BATCH };
            let mut missing: Vec<u32> = server_uids.difference(&local_uids).copied().collect();
            missing.sort_unstable_by(|a, b| b.cmp(a)); // najnowsze najpierw
            remaining = missing.len().saturating_sub(budget);
            missing.truncate(budget);
            // Nic nie zostało do dociągnięcia - folder jest domknięty i od
            // następnego przebiegu sprawdzamy go już tylko tanio.
            if remaining == 0 {
                let db = app.state::<Db>();
                let conn = db.0.lock().unwrap();
                conn.execute("UPDATE folders SET backfilled = 1 WHERE id = ?1", [folder_id])?;
            }
            missing
        };
        missing.sort_unstable_by(|a, b| b.cmp(a));

        // Pasek stanu zapala się tylko wtedy, gdy naprawdę coś ściągamy.
        // Sam przegląd folderów przebiega po cichu - inaczej migałby bez przerwy.
        if !missing.is_empty() {
            let _ = app.emit(
                "sync-status",
                if remaining > 0 {
                    format!("Pobieram: {decoded} ({email}) - zostało {remaining}")
                } else {
                    format!("Pobieram: {decoded} ({email})")
                },
            );
        }

        let query = "(UID FLAGS INTERNALDATE BODY.PEEK[])";
        let mut fetched: Vec<FetchedMessage> = Vec::new();
        for chunk in missing.chunks(FETCH_CHUNK) {
            let set = chunk.iter().map(|u| u.to_string()).collect::<Vec<_>>().join(",");
            let stream = session.uid_fetch(set, query).await?;
            let items = stream.try_collect::<Vec<_>>().await?;
            fetched.extend(items.iter().filter_map(to_fetched));
        }

        // Statusy zmienione po stronie serwera (np. przeczytane w Outlooku).
        // Przy dużych folderach to kosztowna operacja, więc tylko w trybie pełnym.
        if mode == SyncMode::Full {
            if let Err(e) = refresh_flags(app, &mut session, folder_id).await {
                eprintln!("[sync] odświeżanie flag folderu {folder_id}: {e}");
            }
        }

        if fetched.is_empty() {
            // Reguły i tak muszą przejść: mogły dojść nowe albo zmienić się
            // ich definicje od poprzedniego przebiegu.
            match apply_rules(app, &mut session, account_id, folder_id).await {
                Ok(moved) => rule_targets.extend(moved),
                Err(e) => eprintln!("[sync] reguły dla folderu {folder_id}: {e}"),
            }
            continue;
        }
        let max_uid = fetched.iter().map(|m| i64::from(m.uid)).max().unwrap_or(last_seen);

        // Nowe, nieprzeczytane wiadomości ze skrzynki odbiorczej - o nich
        // powiadamiamy po zapisaniu paczki.
        let mut swieze: Vec<crate::notify::NewMail> = Vec::new();
        let is_inbox = selectable
            .iter()
            .find(|(n, ..)| *n == folder_name)
            .map(|(_, _, kind)| *kind == "inbox")
            .unwrap_or(false);
        {
            let db = app.state::<Db>();
            let conn = db.0.lock().unwrap();
            for msg in &fetched {
                let parsed = match mail::parse_email(&msg.raw) {
                    Some(p) => p,
                    None => continue,
                };
                let date = if parsed.date != 0 { parsed.date } else { msg.internal_date };
                let preview = mail::make_preview(
                    parsed.text.as_deref().unwrap_or(&parsed.subject),
                    140,
                );
                let subject_key = mail::subject_key(&parsed.subject);
                let thread_id = resolve_thread(
                    &conn,
                    account_id,
                    &parsed,
                    &subject_key,
                    date,
                    mail::is_reply_subject(&parsed.subject),
                );
                let inserted = conn.execute(
                    "INSERT OR IGNORE INTO messages
                       (folder_id, uid, message_id, in_reply_to, thread_id, subject_key, subject,
                        from_name, from_addr, to_addrs, date, preview, is_read, is_flagged,
                        has_attachments, category)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
                    rusqlite::params![
                        folder_id,
                        msg.uid,
                        parsed.message_id,
                        parsed.refs.first(),
                        thread_id,
                        subject_key,
                        parsed.subject,
                        parsed.from_name,
                        parsed.from_addr,
                        parsed.to_addrs,
                        date,
                        preview,
                        msg.seen as i64,
                        msg.flagged as i64,
                        parsed.has_attachments as i64,
                        parsed.category,
                    ],
                )?;
                if inserted > 0 {
                    let new_id = conn.last_insert_rowid();
                    conn.execute(
                        "INSERT OR REPLACE INTO message_bodies (message_id, html, text) VALUES (?1, ?2, ?3)",
                        rusqlite::params![new_id, parsed.html, parsed.text],
                    )?;
                    // O starej poczcie dociąganej w tle nie powiadamiamy -
                    // tylko o świeżej, nieprzeczytanej, w skrzynce odbiorczej.
                    let swieza = date > chrono::Utc::now().timestamp() - 24 * 3600;
                    if is_inbox && !msg.seen && swieza {
                        swieze.push(crate::notify::NewMail {
                            id: new_id,
                            from: if parsed.from_name.trim().is_empty() {
                                parsed.from_addr.clone()
                            } else {
                                parsed.from_name.clone()
                            },
                            subject: parsed.subject.clone(),
                            category: parsed.category.to_string(),
                        });
                    }
                }
            }
            conn.execute(
                "UPDATE folders SET last_seen_uid = ?2, uid_validity = ?3 WHERE id = ?1",
                rusqlite::params![folder_id, max_uid.max(last_seen), uidv],
            )?;
        }
        crate::notify::new_mail(app, swieze);
        let _ = app.emit("messages-updated", ());

        // Reguły „od nadawcy do folderu" dopiero teraz - dopiero co zapisane
        // wiadomości też mają być przeniesione, a nie czekać na kolejny przebieg.
        match apply_rules(app, &mut session, account_id, folder_id).await {
            Ok(moved) => rule_targets.extend(moved),
            Err(e) => eprintln!("[sync] reguły dla folderu {folder_id}: {e}"),
        }
    }

    // Foldery docelowe reguł doczytujemy od razu, w tym samym przebiegu -
    // inaczej przeniesiona wiadomość znika ze skrzynki i pojawia się w folderze
    // dopiero po pełnej synchronizacji.
    for (folder_id, folder_name) in rule_targets {
        match pull_new(app, &mut session, account_id, folder_id, &folder_name).await {
            Ok(0) => {}
            Ok(_) => {
                let _ = app.emit("messages-updated", ());
            }
            Err(e) => eprintln!("[sync] doczytanie folderu {folder_id}: {e}"),
        }
    }

    session.logout().await.ok();
    let _ = app.emit("messages-updated", ());
    let _ = app.emit("sync-status", "");
    Ok(())
}

/// Pobiera do folderu wiadomości, których jeszcze nie mamy - wariant tani,
/// pytający wyłącznie o UID-y powyżej ostatnio widzianego. Używane po
/// przeniesieniu wiadomości regułą, żeby od razu było ją widać w celu.
async fn pull_new(
    app: &AppHandle,
    session: &mut ImapSession,
    account_id: i64,
    folder_id: i64,
    folder_name: &str,
) -> Result<usize> {
    let mailbox = session.select(folder_name).await?;
    if mailbox.exists == 0 {
        return Ok(0);
    }
    let last_seen: i64 = {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        conn.query_row(
            "SELECT last_seen_uid FROM folders WHERE id = ?1",
            [folder_id],
            |r| r.get(0),
        )?
    };
    let found = session.uid_search(format!("UID {}:*", last_seen + 1)).await?;
    let known: std::collections::HashSet<u32> = {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT uid FROM messages WHERE folder_id = ?1 AND uid > ?2")?;
        let rows = stmt.query_map(rusqlite::params![folder_id, last_seen], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let missing: Vec<u32> = found
        .into_iter()
        .filter(|u| i64::from(*u) > last_seen && !known.contains(u))
        .collect();
    if missing.is_empty() {
        return Ok(0);
    }

    let mut fetched: Vec<FetchedMessage> = Vec::new();
    for chunk in missing.chunks(FETCH_CHUNK) {
        let set = chunk.iter().map(|u| u.to_string()).collect::<Vec<_>>().join(",");
        let stream = session.uid_fetch(set, "(UID FLAGS INTERNALDATE BODY.PEEK[])").await?;
        let items = stream.try_collect::<Vec<_>>().await?;
        fetched.extend(items.iter().filter_map(to_fetched));
    }
    let max_uid = fetched.iter().map(|m| i64::from(m.uid)).max().unwrap_or(last_seen);

    let db = app.state::<Db>();
    let conn = db.0.lock().unwrap();
    let mut stored = 0;
    for msg in &fetched {
        if store_message(&conn, account_id, folder_id, msg)? {
            stored += 1;
        }
    }
    conn.execute(
        "UPDATE folders SET last_seen_uid = ?2 WHERE id = ?1",
        rusqlite::params![folder_id, max_uid.max(last_seen)],
    )?;
    Ok(stored)
}

/// Zapisuje pobraną wiadomość w bazie. Wspólne dla synchronizacji i dla
/// wyszukiwania na serwerze.
fn store_message(
    conn: &rusqlite::Connection,
    account_id: i64,
    folder_id: i64,
    msg: &FetchedMessage,
) -> Result<bool> {
    let Some(parsed) = mail::parse_email(&msg.raw) else {
        return Ok(false);
    };
    let date = if parsed.date != 0 { parsed.date } else { msg.internal_date };
    let preview = mail::make_preview(parsed.text.as_deref().unwrap_or(&parsed.subject), 140);
    let subject_key = mail::subject_key(&parsed.subject);
    let thread_id = resolve_thread(
        conn,
        account_id,
        &parsed,
        &subject_key,
        date,
        mail::is_reply_subject(&parsed.subject),
    );
    let inserted = conn.execute(
        "INSERT OR IGNORE INTO messages
           (folder_id, uid, message_id, in_reply_to, thread_id, subject_key, subject,
            from_name, from_addr, to_addrs, date, preview, is_read, is_flagged,
            has_attachments, category)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        rusqlite::params![
            folder_id,
            msg.uid,
            parsed.message_id,
            parsed.refs.first(),
            thread_id,
            subject_key,
            parsed.subject,
            parsed.from_name,
            parsed.from_addr,
            parsed.to_addrs,
            date,
            preview,
            msg.seen as i64,
            msg.flagged as i64,
            parsed.has_attachments as i64,
            parsed.category,
        ],
    )?;
    if inserted > 0 {
        conn.execute(
            "INSERT OR REPLACE INTO message_bodies (message_id, html, text) VALUES (?1, ?2, ?3)",
            rusqlite::params![conn.last_insert_rowid(), parsed.html, parsed.text],
        )?;
    }
    Ok(inserted > 0)
}

/// Szuka na serwerze wiadomości, których jeszcze nie ma lokalnie, i dociąga je.
/// Dzięki temu wyszukiwarka sięga całej skrzynki, nie tylko pobranej historii.
pub async fn search_on_server(app: &AppHandle, criteria: String) -> Result<usize> {
    /// Ile wiadomości maksymalnie dociągamy w jednym wyszukiwaniu.
    const LIMIT: usize = 200;

    let accounts: Vec<(i64, String, String, String, u16)> = {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, email, login, imap_host, imap_port FROM accounts WHERE auth_kind != 'demo'",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    let mut pobrane = 0usize;
    for (account_id, email, login, host, port) in accounts {
        let login = if login.is_empty() { email.clone() } else { login };
        let Ok(password) = accounts::get_password(&email) else { continue };
        let mut session = connect_session(&host, port, &login, &password).await?;

        let folders: Vec<(i64, String, String)> = {
            let db = app.state::<Db>();
            let conn = db.0.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT id, name, display_name FROM folders WHERE account_id = ?1
                 ORDER BY kind = 'inbox' DESC, display_name",
            )?;
            let rows = stmt.query_map([account_id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };

        for (folder_id, raw_name, display) in folders {
            if pobrane >= LIMIT {
                break;
            }
            let _ = app.emit("sync-status", format!("Szukam na serwerze: {display}"));
            if session.select(&raw_name).await.is_err() {
                continue;
            }
            let Ok(found) = session.uid_search(&criteria).await else { continue };
            if found.is_empty() {
                continue;
            }
            let local: std::collections::HashSet<u32> = {
                let db = app.state::<Db>();
                let conn = db.0.lock().unwrap();
                let mut stmt = conn.prepare("SELECT uid FROM messages WHERE folder_id = ?1")?;
                let rows = stmt.query_map([folder_id], |r| r.get::<_, u32>(0))?;
                rows.collect::<rusqlite::Result<std::collections::HashSet<_>>>()?
            };
            let mut missing: Vec<u32> = found.difference(&local).copied().collect();
            missing.sort_unstable_by(|a, b| b.cmp(a));
            missing.truncate(LIMIT - pobrane);

            for chunk in missing.chunks(FETCH_CHUNK) {
                let set = chunk.iter().map(|u| u.to_string()).collect::<Vec<_>>().join(",");
                let Ok(stream) = session
                    .uid_fetch(set, "(UID FLAGS INTERNALDATE BODY.PEEK[])")
                    .await
                else {
                    continue;
                };
                let items = stream.try_collect::<Vec<_>>().await?;
                let db = app.state::<Db>();
                let conn = db.0.lock().unwrap();
                for msg in items.iter().filter_map(to_fetched) {
                    if store_message(&conn, account_id, folder_id, &msg)? {
                        pobrane += 1;
                    }
                }
            }
        }
        session.logout().await.ok();
    }
    let _ = app.emit("sync-status", "");
    let _ = app.emit("messages-updated", ());
    Ok(pobrane)
}

/// Wysyła na serwer statusy zmienione lokalnie (przeczytane, flaga) dla
/// aktualnie wybranego folderu i czyści znacznik `flags_dirty`.
async fn push_dirty_flags(
    app: &AppHandle,
    session: &mut ImapSession,
    folder_id: i64,
) -> Result<()> {
    let dirty: Vec<(i64, u32, bool, bool)> = {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, uid, is_read, is_flagged FROM messages
             WHERE folder_id = ?1 AND flags_dirty = 1 AND uid > 0",
        )?;
        let rows = stmt.query_map([folder_id], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get::<_, i64>(2)? != 0,
                r.get::<_, i64>(3)? != 0,
            ))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    if dirty.is_empty() {
        return Ok(());
    }

    // Cztery zbiory UID-ów: dodaj/usuń \Seen oraz \Flagged.
    let uids = |pick: &dyn Fn(&(i64, u32, bool, bool)) -> bool| -> String {
        dirty
            .iter()
            .filter(|m| pick(m))
            .map(|m| m.1.to_string())
            .collect::<Vec<_>>()
            .join(",")
    };
    let operations = [
        (uids(&|m| m.2), "+FLAGS.SILENT (\\Seen)"),
        (uids(&|m| !m.2), "-FLAGS.SILENT (\\Seen)"),
        (uids(&|m| m.3), "+FLAGS.SILENT (\\Flagged)"),
        (uids(&|m| !m.3), "-FLAGS.SILENT (\\Flagged)"),
    ];
    for (set, query) in operations {
        if set.is_empty() {
            continue;
        }
        let stream = session.uid_store(&set, query).await?;
        stream.try_collect::<Vec<_>>().await?;
    }

    let db = app.state::<Db>();
    let conn = db.0.lock().unwrap();
    for (id, ..) in &dirty {
        conn.execute("UPDATE messages SET flags_dirty = 0 WHERE id = ?1", [id])?;
    }
    Ok(())
}

/// Przenosi wiadomości pasujące do reguł konta z bieżącego folderu do folderu
/// docelowego (po stronie serwera). Lokalne wiersze kasujemy - kolejny przebieg
/// pobierze te wiadomości już w nowym folderze.
async fn apply_rules(
    app: &AppHandle,
    session: &mut ImapSession,
    account_id: i64,
    folder_id: i64,
) -> Result<Vec<(i64, String)>> {
    let rules: Vec<(String, i64, String)> = {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT json_extract(r.conditions_json, '$.from'),
                    f.id, f.name
             FROM rules r JOIN folders f ON f.id = json_extract(r.actions_json, '$.moveTo')
             WHERE r.account_id = ?1 AND r.enabled = 1 AND f.id != ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![account_id, folder_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    if rules.is_empty() {
        return Ok(Vec::new());
    }

    let mut moved = Vec::new();
    for (from_addr, target_id, target_name) in rules {
        let matched: Vec<(i64, u32)> = {
            let db = app.state::<Db>();
            let conn = db.0.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT id, uid FROM messages
                 WHERE folder_id = ?1 AND uid > 0 AND lower(from_addr) LIKE ?2",
            )?;
            let rows = stmt.query_map(rusqlite::params![folder_id, format!("%{from_addr}%")], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        if matched.is_empty() {
            continue;
        }
        let uid_set = matched
            .iter()
            .map(|(_, uid)| uid.to_string())
            .collect::<Vec<_>>()
            .join(",");
        session.uid_mv(&uid_set, &target_name).await?;

        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        for (id, _) in &matched {
            conn.execute("DELETE FROM messages WHERE id = ?1", [id])?;
        }
        drop(conn);
        moved.push((target_id, target_name));
        let _ = app.emit("messages-updated", ());
    }
    Ok(moved)
}

/// Pobiera z serwera aktualne flagi wiadomości i wyrównuje lokalny stan.
/// Wiersze z niewysłanymi zmianami (`flags_dirty`) pomijamy - mają pierwszeństwo.
async fn refresh_flags(
    app: &AppHandle,
    session: &mut ImapSession,
    folder_id: i64,
) -> Result<()> {
    // Tylko ostatnie `FLAG_WINDOW` wiadomości. Ciągnięcie flag całej skrzynki
    // (`1:*`) przy kilkunastu tysiącach listów trwa dłużej niż cały pozostały
    // przebieg, a starej poczty i tak nikt już nie odznacza.
    let from: u32 = {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        conn.query_row(
            "SELECT uid FROM messages WHERE folder_id = ?1 AND uid > 0
             ORDER BY uid DESC LIMIT 1 OFFSET ?2",
            rusqlite::params![folder_id, FLAG_WINDOW - 1],
            |r| r.get(0),
        )
        .unwrap_or(1)
    };
    let stream = session.uid_fetch(format!("{from}:*"), "(UID FLAGS)").await?;
    let items: Vec<(u32, bool, bool)> = stream
        .try_collect::<Vec<_>>()
        .await?
        .iter()
        .filter_map(|f| {
            Some((
                f.uid?,
                f.flags().any(|fl| matches!(fl, Flag::Seen)),
                f.flags().any(|fl| matches!(fl, Flag::Flagged)),
            ))
        })
        .collect();
    if items.is_empty() {
        return Ok(());
    }
    let db = app.state::<Db>();
    let conn = db.0.lock().unwrap();
    for (uid, seen, flagged) in items {
        conn.execute(
            "UPDATE messages SET is_read = ?3, is_flagged = ?4
             WHERE folder_id = ?1 AND uid = ?2 AND flags_dirty = 0
               AND (is_read != ?3 OR is_flagged != ?4)",
            rusqlite::params![folder_id, uid, seen as i64, flagged as i64],
        )?;
    }
    Ok(())
}

/// Okno czasowe grupowania po samym temacie (30 dni) - bez niego cykliczne
/// raporty o stałym tytule skleiłyby się w jeden nieskończony wątek.
const SUBJECT_THREAD_WINDOW: i64 = 30 * 24 * 3600;

/// Wyznacza wątek wiadomości: najpierw po nagłówkach References/In-Reply-To
/// (dokładne), potem - tylko dla odpowiedzi - po temacie w oknie czasowym.
///
/// Dopasowanie po samym temacie dotyczy wyłącznie wiadomości wyglądających na
/// odpowiedź („Re:", „Odp:", „Fwd:"). Nowa wiadomość zaczyna własny wątek,
/// choćby ktoś już kiedyś wysłał coś o tym samym tytule. Inaczej każdy kolejny
/// mail o temacie „test" doklejał się do poprzedniego, a okno 30 dni tego nie
/// ratowało, bo liczy się od najbliższej wiadomości w wątku - łańcuch
/// przesuwał się w nieskończoność i mieszał korespondencję z różnymi osobami.
fn resolve_thread(
    conn: &rusqlite::Connection,
    account_id: i64,
    parsed: &mail::ParsedEmail,
    subject_key: &str,
    date: i64,
    is_reply: bool,
) -> String {
    for id in &parsed.refs {
        let found: Option<String> = conn
            .query_row(
                "SELECT m.thread_id FROM messages m
                 JOIN folders f ON f.id = m.folder_id
                 WHERE f.account_id = ?1 AND m.message_id = ?2 AND m.thread_id IS NOT NULL
                 LIMIT 1",
                rusqlite::params![account_id, id],
                |r| r.get(0),
            )
            .ok();
        if let Some(thread) = found {
            return thread;
        }
    }
    if !subject_key.is_empty() && is_reply {
        let found: Option<String> = conn
            .query_row(
                "SELECT m.thread_id FROM messages m
                 JOIN folders f ON f.id = m.folder_id
                 WHERE f.account_id = ?1 AND m.subject_key = ?2
                   AND abs(m.date - ?3) < ?4 AND m.thread_id IS NOT NULL
                 ORDER BY abs(m.date - ?3) LIMIT 1",
                rusqlite::params![account_id, subject_key, date, SUBJECT_THREAD_WINDOW],
                |r| r.get(0),
            )
            .ok();
        if let Some(thread) = found {
            return thread;
        }
    }
    if !subject_key.is_empty() {
        // Własny wątek: identyfikator wiadomości w kluczu, żeby dwie wysłane
        // w tej samej sekundzie nie wylądowały razem.
        let unique = parsed.message_id.as_deref().unwrap_or("");
        return format!("s:{account_id}:{subject_key}:{date}:{unique}");
    }
    match &parsed.message_id {
        Some(id) => format!("m:{id}"),
        None => format!("u:{account_id}:{date}"),
    }
}

/// Synchronizuje wszystkie prawdziwe konta (pomija demo).
pub async fn sync_all(app: &AppHandle) {
    sync_all_mode(app, SyncMode::Full).await
}

pub async fn sync_all_mode(app: &AppHandle, mode: SyncMode) {
    let ids: Vec<i64> = {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        let mut stmt = match conn.prepare("SELECT id FROM accounts WHERE auth_kind != 'demo'") {
            Ok(s) => s,
            Err(_) => return,
        };
        stmt.query_map([], |r| r.get(0))
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default()
    };
    for id in ids {
        // Porzucenie future'a zamyka gniazdo, więc następny przebieg zaczyna
        // od świeżego połączenia zamiast dobijać się do martwego.
        let budget = match mode {
            SyncMode::Quick => QUICK_TIMEOUT,
            SyncMode::Full => FULL_TIMEOUT,
        };
        match tokio::time::timeout(budget, sync_account_mode(app, id, mode)).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                eprintln!("[sync] konto {id}: {e}");
                let _ = app.emit("sync-error", format!("{e}"));
                let _ = app.emit("sync-status", "");
            }
            Err(_) => {
                // Przerwany przebieg nie jest błędem użytkownika: następny
                // podejmie pracę tam, gdzie ten skończył. Krzyczymy więc tylko
                // przy trybie szybkim, gdzie przekroczenie naprawdę oznacza
                // kłopot - przy pełnym zostaje wpis w logu.
                let secs = budget.as_secs();
                eprintln!("[sync] konto {id}: przekroczono {secs} s - przerywam przebieg");
                if matches!(mode, SyncMode::Quick) {
                    let _ = app.emit(
                        "sync-error",
                        format!("konto nie odpowiada od {secs} s - przerwano przebieg"),
                    );
                }
                let _ = app.emit("sync-status", "");
            }
        }
    }
}

/// Pętla działająca w tle przez cały czas życia aplikacji:
/// - co 30 s: drzemki, kolejka wysyłki i zaglądnięcie do skrzynek odbiorczych
///   po nową pocztę (szybkie, więc mail dociera niemal od razu),
/// - co 10 min: pełny przebieg - wszystkie foldery, statusy z serwera
///   i dociąganie zaległej historii.
pub fn start_scheduler(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut ticks: u64 = 0;
        loop {
            if let Err(e) = tick(&app) {
                eprintln!("[scheduler] błąd: {e}");
            }
            crate::send::process_outbox(&app).await;
            let mode = if ticks % 20 == 0 { SyncMode::Full } else { SyncMode::Quick };
            sync_all_mode(&app, mode).await;
            ticks += 1;
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    });
}

fn tick(app: &AppHandle) -> Result<()> {
    let db = app.state::<Db>();
    let conn = db.0.lock().unwrap();
    let now = chrono::Utc::now().timestamp();
    let woken = conn.execute(
        "UPDATE messages SET snoozed_until = NULL, is_read = 0
         WHERE snoozed_until IS NOT NULL AND snoozed_until <= ?1",
        [now],
    )?;
    drop(conn);
    if woken > 0 {
        let _ = app.emit("messages-updated", ());
    }
    Ok(())
}
