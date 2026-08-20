//! Komendy Tauri - API, przez które frontend rozmawia z rdzeniem Rust.
//! Wszystkie odczyty idą z lokalnego SQLite (offline-first), nigdy z sieci.

use crate::db::Db;
use crate::error::{AppError, Result};
use crate::models::*;
use crate::{accounts, sync};
use futures_util::TryStreamExt;
use rusqlite::{params, Row};
use tauri::{AppHandle, Emitter, Manager, State};

const SUMMARY_COLS: &str = "m.id, m.folder_id, m.subject, m.from_name, m.from_addr, m.date, \
     m.preview, m.is_read, m.is_flagged, m.has_attachments, m.category, m.snoozed_until, \
     COALESCE(m.thread_id, '')";

fn row_to_summary(row: &Row) -> rusqlite::Result<MessageSummary> {
    Ok(MessageSummary {
        id: row.get(0)?,
        folder_id: row.get(1)?,
        subject: row.get(2)?,
        from_name: row.get(3)?,
        from_addr: row.get(4)?,
        date: row.get(5)?,
        preview: row.get(6)?,
        is_read: row.get::<_, i64>(7)? != 0,
        is_flagged: row.get::<_, i64>(8)? != 0,
        has_attachments: row.get::<_, i64>(9)? != 0,
        category: row.get(10)?,
        snoozed_until: row.get(11)?,
        thread_id: row.get(12)?,
        thread_count: row.get::<_, Option<i64>>(13).unwrap_or(None).unwrap_or(1),
        thread_unread: row.get::<_, Option<i64>>(14).unwrap_or(None).unwrap_or(0),
    })
}

#[tauri::command]
pub async fn list_accounts(db: State<'_, Db>) -> Result<Vec<Account>> {
    let conn = db.0.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT id, email, display_name, sender_name, imap_host, imap_port, smtp_host, smtp_port, auth_kind
         FROM accounts ORDER BY id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(Account {
            id: r.get(0)?,
            email: r.get(1)?,
            display_name: r.get(2)?,
            sender_name: r.get(3)?,
            imap_host: r.get(4)?,
            imap_port: r.get(5)?,
            smtp_host: r.get(6)?,
            smtp_port: r.get(7)?,
            auth_kind: r.get(8)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

#[tauri::command]
pub async fn add_account(db: State<'_, Db>, new_account: NewAccount) -> Result<Account> {
    if let Some(password) = &new_account.password {
        accounts::store_password(&new_account.email, password)?;
    }
    let conn = db.0.lock().unwrap();
    conn.execute(
        "INSERT INTO accounts (email, display_name, sender_name, login, imap_host, imap_port, smtp_host, smtp_port, auth_kind)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            new_account.email,
            new_account.display_name,
            new_account.sender_name,
            new_account.login,
            new_account.imap_host,
            new_account.imap_port,
            new_account.smtp_host,
            new_account.smtp_port,
            new_account.auth_kind,
        ],
    )?;
    let id = conn.last_insert_rowid();
    // Placeholder do czasu pierwszej synchronizacji, żeby UI miał gdzie wskazać.
    conn.execute(
        "INSERT INTO folders (account_id, name, display_name, kind) VALUES (?1, 'INBOX', 'Odebrane', 'inbox')",
        [id],
    )?;
    Ok(Account {
        id,
        email: new_account.email,
        display_name: new_account.display_name,
        sender_name: new_account.sender_name,
        imap_host: new_account.imap_host,
        imap_port: new_account.imap_port,
        smtp_host: new_account.smtp_host,
        smtp_port: new_account.smtp_port,
        auth_kind: new_account.auth_kind,
    })
}

#[tauri::command]
pub async fn remove_account(db: State<'_, Db>, id: i64) -> Result<()> {
    let email: String = {
        let conn = db.0.lock().unwrap();
        let email = conn.query_row("SELECT email FROM accounts WHERE id = ?1", [id], |r| r.get(0))?;
        conn.execute("DELETE FROM accounts WHERE id = ?1", [id])?;
        email
    };
    accounts::delete_password(&email)?;
    // OAuth2 accounts carry a second keychain entry; leaving it behind would
    // hand a re-added account someone else's stale refresh token.
    accounts::delete_refresh_token(&email).ok();
    crate::auth::forget(&email);
    Ok(())
}

/// Runs the Microsoft sign-in and stores the refresh token for this address.
///
/// The account row itself is written afterwards by `add_account` - keeping the
/// two apart means a cancelled or failed sign-in leaves nothing behind.
#[tauri::command]
pub async fn oauth_sign_in(app: AppHandle, email: String) -> Result<()> {
    let client_id = crate::auth::client_id(&app)?;
    let tokens = crate::oauth::login(&client_id).await?;
    // Okno logowania Microsoftu pamięta sesje, więc łatwo kliknąć nie to konto,
    // co trzeba. Bez tej kontroli tokeny wylądowałyby w pęku kluczy pod cudzym
    // adresem, a IMAP odmawiałby logowania bez słowa wyjaśnienia.
    if let Some(signed_in) = tokens.account.as_deref() {
        if !signed_in.eq_ignore_ascii_case(email.trim()) {
            return Err(AppError::Other(format!(
                "zalogowano się jako {signed_in}, a konto dodajesz dla {}: zaloguj się na właściwe konto",
                email.trim()
            )));
        }
    }
    if tokens.refresh_token.is_empty() {
        return Err(AppError::Other(
            "Microsoft did not return a refresh token - check that the application requests offline_access".into(),
        ));
    }
    crate::auth::remember(&email, tokens)?;
    Ok(())
}

/// Whether an application ID has been set, so the interface can offer the
/// Microsoft button instead of failing at the moment it is pressed.
#[tauri::command]
pub async fn oauth_is_configured(app: AppHandle) -> Result<bool> {
    Ok(crate::auth::client_id(&app).is_ok())
}

#[tauri::command]
pub async fn list_folders(db: State<'_, Db>, account_id: Option<i64>) -> Result<Vec<Folder>> {
    let conn = db.0.lock().unwrap();
    // Oba liczniki jednym przejściem po indeksie (folder_id, is_read) - osobne
    // podzapytania czytałyby te same wiersze dwa razy przy każdym odświeżeniu.
    let sql = "SELECT f.id, f.account_id, f.name, f.display_name, f.kind,
                COALESCE(c.unread, 0), COALESCE(c.total, 0)
               FROM folders f
               LEFT JOIN (
                 SELECT folder_id,
                        SUM(CASE WHEN is_read = 0 THEN 1 ELSE 0 END) AS unread,
                        COUNT(*) AS total
                 FROM messages WHERE snoozed_until IS NULL GROUP BY folder_id
               ) c ON c.folder_id = f.id
               WHERE (?1 IS NULL OR f.account_id = ?1)
               ORDER BY f.account_id, f.sort_order, f.kind = 'inbox' DESC, f.display_name";
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params![account_id], |r| {
        Ok(Folder {
            id: r.get(0)?,
            account_id: r.get(1)?,
            name: r.get(2)?,
            display_name: r.get(3)?,
            kind: r.get(4)?,
            unread_count: r.get(5)?,
            total_count: r.get(6)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Lista wiadomości. `folder_id = None` to zunifikowana skrzynka odbiorcza
/// (wszystkie foldery typu inbox ze wszystkich kont). `category` zawęża do
/// zakładki Smart Inbox ("primary" | "newsletters" | "notifications").
/// Kolejność listy. Nazwa przychodzi z interfejsu, ale do SQL trafia wyłącznie
/// stała z tej funkcji - zapytania nie da się w ten sposób podmienić.
fn order_clause(sort: Option<&str>) -> &'static str {
    match sort.unwrap_or("date_desc") {
        "date_asc" => "max_date ASC",
        // Przy sortowaniu po nadawcy i temacie data zostaje drugim kryterium,
        // żeby wiadomości od jednej osoby układały się od najnowszej.
        "from" => "lower(COALESCE(NULLIF(from_name, ''), from_addr)) ASC, max_date DESC",
        "subject" => "lower(subject) ASC, max_date DESC",
        "unread" => "(unread > 0) DESC, max_date DESC",
        "attachments" => "has_attachments DESC, max_date DESC",
        _ => "max_date DESC",
    }
}

#[tauri::command]
pub async fn list_messages(
    db: State<'_, Db>,
    folder_id: Option<i64>,
    category: Option<String>,
    sort: Option<String>,
    offset: i64,
    limit: i64,
) -> Result<Vec<MessageSummary>> {
    let conn = db.0.lock().unwrap();
    // Lista pokazuje konwersacje: jeden wiersz na wątek - najnowsza wiadomość
    // plus liczba wiadomości i nieprzeczytanych w całym wątku.
    // Jedno przejście po danych: funkcje okna liczą rozmiar wątku i wybierają
    // jego najnowszą wiadomość. Wcześniejsza wersja wołała podzapytanie dla
    // każdego wątku osobno, co przy tysiącach maili wyraźnie spowalniało
    // przełączanie folderów.
    let sql = format!(
        "WITH filtered AS (
             SELECT m.*, COALESCE(m.thread_id, 'm:' || m.id) AS tid
             FROM messages m JOIN folders f ON f.id = m.folder_id
             WHERE m.snoozed_until IS NULL
               AND (?1 IS NULL OR m.folder_id = ?1)
               AND (?1 IS NOT NULL OR f.kind = 'inbox')
               AND (?2 IS NULL OR m.category = ?2)
         ),
         ranked AS (
             SELECT filtered.*,
                    ROW_NUMBER() OVER (PARTITION BY tid ORDER BY date DESC, id DESC) AS rn,
                    COUNT(*) OVER (PARTITION BY tid) AS cnt,
                    SUM(CASE WHEN is_read = 0 THEN 1 ELSE 0 END) OVER (PARTITION BY tid) AS unread,
                    MAX(date) OVER (PARTITION BY tid) AS max_date
             FROM filtered
         )
         SELECT id, folder_id, subject, from_name, from_addr, date, preview, is_read,
                is_flagged, has_attachments, category, snoozed_until,
                COALESCE(thread_id, ''), cnt, unread
         FROM ranked WHERE rn = 1
         ORDER BY {}
         LIMIT ?3 OFFSET ?4",
        order_clause(sort.as_deref())
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![folder_id, category, limit, offset], row_to_summary)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Nieprzeczytane w każdej zakładce Smart Inbox. Zakres liczenia jest ten sam
/// co w `list_messages`: wybrany folder albo wszystkie skrzynki odbiorcze.
/// Liczymy wiadomości, nie wątki - tak samo jak licznik przy folderze.
#[tauri::command]
pub async fn category_counts(db: State<'_, Db>, folder_id: Option<i64>) -> Result<CategoryCounts> {
    let conn = db.0.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT m.category, COUNT(*)
         FROM messages m JOIN folders f ON f.id = m.folder_id
         WHERE m.is_read = 0 AND m.snoozed_until IS NULL
           AND (?1 IS NULL OR m.folder_id = ?1)
           AND (?1 IS NOT NULL OR f.kind = 'inbox')
         GROUP BY m.category",
    )?;
    let rows = stmt.query_map(params![folder_id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })?;
    let mut counts = CategoryCounts::default();
    for row in rows {
        let (category, n) = row?;
        match category.as_str() {
            "newsletters" => counts.newsletters = n,
            "notifications" => counts.notifications = n,
            _ => counts.primary = n,
        }
    }
    Ok(counts)
}

/// Zamienia zapytanie użytkownika na kryteria wyszukiwania IMAP.
fn imap_criteria(query: &str) -> String {
    let q = parse_search(query);
    let quote = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    let mut parts: Vec<String> = Vec::new();
    for t in &q.text {
        parts.push(format!("TEXT {}", quote(t)));
    }
    for v in &q.from {
        parts.push(format!("FROM {}", quote(v)));
    }
    for v in &q.to {
        parts.push(format!("TO {}", quote(v)));
    }
    for v in &q.subject {
        parts.push(format!("SUBJECT {}", quote(v)));
    }
    if q.unread {
        parts.push("UNSEEN".into());
    }
    if q.flagged {
        parts.push("FLAGGED".into());
    }
    let date_fmt = |ts: i64| {
        chrono::DateTime::from_timestamp(ts, 0)
            .map(|d| d.format("%d-%b-%Y").to_string())
            .unwrap_or_default()
    };
    if let Some(after) = q.after {
        parts.push(format!("SINCE {}", date_fmt(after)));
    }
    if let Some(before) = q.before {
        parts.push(format!("BEFORE {}", date_fmt(before)));
    }
    if parts.is_empty() {
        return String::new();
    }
    let joined = parts.join(" ");
    // Polskie znaki wymagają zadeklarowania kodowania.
    if joined.is_ascii() {
        joined
    } else {
        format!("CHARSET UTF-8 {joined}")
    }
}

/// Szuka na serwerze wiadomości spoza pobranej historii i dociąga znalezione.
#[tauri::command]
pub async fn search_server(app: AppHandle, query: String) -> Result<usize> {
    let criteria = imap_criteria(&query);
    if criteria.is_empty() {
        return Ok(0);
    }
    sync::search_on_server(&app, criteria).await
}

/// Pojedyncza wiadomość - używane po kliknięciu w powiadomienie.
#[tauri::command]
pub async fn get_message(db: State<'_, Db>, id: i64) -> Result<MessageSummary> {
    let conn = db.0.lock().unwrap();
    let sql = format!("SELECT {SUMMARY_COLS}, 1, 0 FROM messages m WHERE m.id = ?1");
    conn.query_row(&sql, [id], row_to_summary).map_err(AppError::from)
}

/// Wszystkie wiadomości jednego wątku (także z innych folderów, np. Wysłane),
/// od najstarszej do najnowszej.
#[tauri::command]
pub async fn list_thread(db: State<'_, Db>, thread_id: String) -> Result<Vec<MessageSummary>> {
    let conn = db.0.lock().unwrap();
    let sql = format!(
        "SELECT {SUMMARY_COLS}, 1, 0 FROM messages m
         WHERE COALESCE(m.thread_id, 'm:' || m.id) = ?1
         ORDER BY m.date ASC, m.id ASC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([thread_id], row_to_summary)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

#[tauri::command]
pub async fn list_snoozed(db: State<'_, Db>) -> Result<Vec<MessageSummary>> {
    let conn = db.0.lock().unwrap();
    let sql = format!(
        "SELECT {SUMMARY_COLS} FROM messages m
         WHERE m.snoozed_until IS NOT NULL
         ORDER BY m.snoozed_until ASC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], row_to_summary)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

#[tauri::command]
pub async fn get_message_body(db: State<'_, Db>, id: i64) -> Result<MessageBody> {
    let mut body = {
        let conn = db.0.lock().unwrap();
        conn.query_row(
            "SELECT m.id, m.to_addrs, b.html, b.text, m.message_id, m.in_reply_to
             FROM messages m LEFT JOIN message_bodies b ON b.message_id = m.id
             WHERE m.id = ?1",
            [id],
            |r| {
                Ok(MessageBody {
                    id: r.get(0)?,
                    to_addrs: r.get(1)?,
                    html: r.get(2)?,
                    text: r.get(3)?,
                    message_id: r.get(4)?,
                    in_reply_to: r.get(5)?,
                })
            },
        )
        .map_err(AppError::from)?
    };
    // Sanityzacja przy wyświetlaniu - baza trzyma surowy HTML.
    body.html = body.html.map(|h| crate::mail::sanitize_html(&h));
    Ok(body)
}

/// Zmiany statusów oznaczamy jako „do wysłania" - harmonogram przekaże je
/// na serwer IMAP, żeby stan zgadzał się z Outlookiem i przetrwał odświeżenie.
#[tauri::command]
pub async fn set_read(db: State<'_, Db>, id: i64, read: bool) -> Result<()> {
    // Log-pułapka: podczas prac nad szkieletem raz wystąpiło masowe oznaczenie
    // wiadomości jako przeczytane (niereprodukowalne po instrumentacji, tylko
    // tryb dev z HMR). Jeśli wróci, ten log wskaże moment i id.
    eprintln!("[set_read] id={id} read={read}");
    let conn = db.0.lock().unwrap();
    conn.execute(
        "UPDATE messages SET is_read = ?2, flags_dirty = 1 WHERE id = ?1",
        params![id, read as i64],
    )?;
    Ok(())
}

/// Oznacza całą konwersację. Panel czytania pokazuje wszystkie wiadomości
/// wątku naraz, więc po otwarciu nie może zostać w nim nic nieprzeczytanego -
/// inaczej licznik przy zakładce liczy maile, których na liście nie widać
/// (wiersz wątku pokazuje stan najnowszej wiadomości).
#[tauri::command]
pub async fn set_thread_read(db: State<'_, Db>, thread_id: String, read: bool) -> Result<usize> {
    if thread_id.is_empty() {
        return Ok(0);
    }
    let conn = db.0.lock().unwrap();
    let changed = conn.execute(
        "UPDATE messages SET is_read = ?2, flags_dirty = 1
         WHERE thread_id = ?1 AND is_read != ?2",
        params![thread_id, read as i64],
    )?;
    Ok(changed)
}

/// Oznacza jako przeczytane wszystko w folderze (albo we wszystkich skrzynkach
/// odbiorczych, gdy `folder_id` jest puste). Zmiany trafiają też na serwer.
#[tauri::command]
pub async fn mark_folder_read(db: State<'_, Db>, folder_id: Option<i64>) -> Result<usize> {
    let conn = db.0.lock().unwrap();
    let changed = conn.execute(
        "UPDATE messages SET is_read = 1, flags_dirty = 1
         WHERE is_read = 0 AND snoozed_until IS NULL
           AND folder_id IN (SELECT id FROM folders
                             WHERE (?1 IS NULL AND kind = 'inbox') OR id = ?1)",
        params![folder_id],
    )?;
    Ok(changed)
}

#[tauri::command]
pub async fn set_flagged(db: State<'_, Db>, id: i64, flagged: bool) -> Result<()> {
    let conn = db.0.lock().unwrap();
    conn.execute(
        "UPDATE messages SET is_flagged = ?2, flags_dirty = 1 WHERE id = ?1",
        params![id, flagged as i64],
    )?;
    Ok(())
}

/// Spark: odłóż wiadomość na później. `until` to unix epoch - harmonogram
/// w `sync::start_scheduler` przywróci ją do skrzynki po terminie.
#[tauri::command]
pub async fn snooze_message(db: State<'_, Db>, id: i64, until: Option<i64>) -> Result<()> {
    let conn = db.0.lock().unwrap();
    conn.execute("UPDATE messages SET snoozed_until = ?2 WHERE id = ?1", params![id, until])?;
    Ok(())
}

/// Usuwa wiadomość także na serwerze: przenosi do Kosza, a jeśli już tam jest
/// (albo konto nie ma Kosza) - kasuje trwale. Bez tego mail wracał przy
/// kolejnej synchronizacji.
#[tauri::command]
pub async fn delete_message(app: AppHandle, id: i64) -> Result<()> {
    let (uid, folder_name, folder_kind, account_id) = {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        conn.query_row(
            "SELECT m.uid, f.name, f.kind, f.account_id
             FROM messages m JOIN folders f ON f.id = m.folder_id
             WHERE m.id = ?1",
            [id],
            |r| {
                Ok((
                    r.get::<_, u32>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            },
        )?
    };

    if uid > 0 {
        let (email, login, host, port, trash) = {
            let db = app.state::<Db>();
            let conn = db.0.lock().unwrap();
            let account = conn.query_row(
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
            let trash: Option<String> = conn
                .query_row(
                    "SELECT name FROM folders WHERE account_id = ?1 AND kind = 'trash' LIMIT 1",
                    [account_id],
                    |r| r.get(0),
                )
                .ok();
            (account.0, account.1, account.2, account.3, trash)
        };
        let login = if login.is_empty() { email.clone() } else { login };
        let password = crate::accounts::get_password(&email)?;
        let mut session = crate::sync::connect_session(&host, port, &login, &password).await?;
        session.select(&folder_name).await?;

        let uid_set = uid.to_string();
        let result = match &trash {
            // Z Kosza (i gdy Kosza brak) usuwamy nieodwracalnie.
            Some(target) if folder_kind != "trash" => session.uid_mv(&uid_set, target).await,
            _ => {
                let stream = session
                    .uid_store(&uid_set, "+FLAGS.SILENT (\\Deleted)")
                    .await?;
                stream.try_collect::<Vec<_>>().await?;
                match session.expunge().await {
                    Ok(stream) => stream.try_collect::<Vec<_>>().await.map(|_| ()),
                    Err(e) => Err(e),
                }
            }
        };
        session.logout().await.ok();
        result.map_err(|e| AppError::Other(format!("serwer odrzucił usunięcie: {e}")))?;
    }

    let db = app.state::<Db>();
    let conn = db.0.lock().unwrap();
    conn.execute("DELETE FROM messages WHERE id = ?1", [id])?;
    Ok(())
}

/// Rozłożone zapytanie wyszukiwarki: operatory + wolny tekst.
#[derive(Default)]
struct SearchQuery {
    text: Vec<String>,
    from: Vec<String>,
    to: Vec<String>,
    subject: Vec<String>,
    folder: Vec<String>,
    unread: bool,
    read: bool,
    flagged: bool,
    has_attachment: bool,
    after: Option<i64>,
    before: Option<i64>,
}

/// Zamienia „2026-08-01" na unix epoch (północ czasu lokalnego jest tu zbędna,
/// wystarczy UTC - filtr jest zgrubny).
fn parse_date(value: &str) -> Option<i64> {
    let d = chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()?;
    Some(d.and_hms_opt(0, 0, 0)?.and_utc().timestamp())
}

/// Rozbiera zapytanie na operatory (`od:`, `do:`, `temat:`, `folder:`,
/// `jest:nieprzeczytane|oflagowane`, `ma:zalacznik`, `po:`, `przed:`)
/// oraz wolny tekst. Akceptuje też angielskie odpowiedniki.
/// Operatory rozpoznawane przez wyszukiwarkę - także po to, żeby wiedzieć,
/// kiedy `od:` z pustą wartością ma sięgnąć po następne słowo.
const SEARCH_KEYS: &[&str] = &[
    "od", "from", "do", "to", "temat", "subject", "tytul", "folder", "in", "jest", "is", "ma",
    "has", "po", "after", "od_daty", "przed", "before", "do_daty",
];

fn parse_search(query: &str) -> SearchQuery {
    let mut q = SearchQuery::default();
    let tokens: Vec<&str> = query.split_whitespace().collect();
    let mut i = 0;
    while i < tokens.len() {
        let token = tokens[i];
        i += 1;
        let (key, value) = match token.split_once(':') {
            Some((k, v)) if !v.is_empty() => (k.to_lowercase(), v.to_string()),
            // `od: nazwisko` - dwukropek na końcu słowa. Naturalnie się tak
            // pisze, a bez tego operator rozpadał się na zwykły tekst
            // i wyszukiwanie pełnotekstowe zwracało każdego, kto wspomniał
            // szukaną osobę w treści.
            Some((k, "")) if SEARCH_KEYS.contains(&k.to_lowercase().as_str()) => {
                match tokens.get(i) {
                    Some(next) => {
                        i += 1;
                        (k.to_lowercase(), (*next).to_string())
                    }
                    None => continue,
                }
            }
            _ => {
                q.text.push(token.to_string());
                continue;
            }
        };
        let value_lower = value.to_lowercase();
        match key.as_str() {
            "od" | "from" => q.from.push(value_lower),
            "do" | "to" => q.to.push(value_lower),
            "temat" | "subject" | "tytul" => q.subject.push(value_lower),
            "folder" | "in" => q.folder.push(value_lower),
            "jest" | "is" => match value_lower.as_str() {
                "nieprzeczytane" | "nieprzeczytany" | "unread" | "nowe" => q.unread = true,
                "przeczytane" | "read" => q.read = true,
                "oflagowane" | "flagged" | "flaga" => q.flagged = true,
                _ => q.text.push(token.to_string()),
            },
            "ma" | "has" => match value_lower.as_str() {
                "zalacznik" | "załącznik" | "attachment" | "zal" => q.has_attachment = true,
                _ => q.text.push(token.to_string()),
            },
            "po" | "after" | "od_daty" => q.after = parse_date(&value),
            "przed" | "before" | "do_daty" => q.before = parse_date(&value),
            _ => q.text.push(token.to_string()),
        }
    }
    q
}

/// Wyszukiwanie pełnotekstowe (FTS5) po nagłówkach i treści wiadomości,
/// z obsługą operatorów. Frazy użytkownika są cytowane, żeby znaki specjalne
/// składni FTS nie wywalały zapytania.
#[tauri::command]
pub async fn search_messages(db: State<'_, Db>, query: String) -> Result<Vec<MessageSummary>> {
    let q = parse_search(&query);
    let mut conditions: Vec<String> = Vec::new();
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if !q.text.is_empty() {
        let fts: String = q
            .text
            .iter()
            .map(|t| format!("\"{}\"*", t.replace('"', "")))
            .collect::<Vec<_>>()
            .join(" ");
        conditions.push(
            "(m.id IN (SELECT rowid FROM messages_fts WHERE messages_fts MATCH ?)
              OR m.id IN (SELECT rowid FROM bodies_fts WHERE bodies_fts MATCH ?))"
                .to_string(),
        );
        args.push(Box::new(fts.clone()));
        args.push(Box::new(fts));
    }
    for value in &q.from {
        conditions.push("(lower(m.from_addr) LIKE ? OR lower(m.from_name) LIKE ?)".into());
        args.push(Box::new(format!("%{value}%")));
        args.push(Box::new(format!("%{value}%")));
    }
    for value in &q.to {
        conditions.push("lower(m.to_addrs) LIKE ?".into());
        args.push(Box::new(format!("%{value}%")));
    }
    for value in &q.subject {
        conditions.push("lower(m.subject) LIKE ?".into());
        args.push(Box::new(format!("%{value}%")));
    }
    for value in &q.folder {
        conditions.push(
            "m.folder_id IN (SELECT id FROM folders WHERE lower(display_name) LIKE ?)".into(),
        );
        args.push(Box::new(format!("%{value}%")));
    }
    if q.unread {
        conditions.push("m.is_read = 0".into());
    }
    if q.read {
        conditions.push("m.is_read = 1".into());
    }
    if q.flagged {
        conditions.push("m.is_flagged = 1".into());
    }
    if q.has_attachment {
        conditions.push("m.has_attachments = 1".into());
    }
    if let Some(after) = q.after {
        conditions.push("m.date >= ?".into());
        args.push(Box::new(after));
    }
    if let Some(before) = q.before {
        conditions.push("m.date < ?".into());
        args.push(Box::new(before));
    }
    if conditions.is_empty() {
        return Ok(vec![]);
    }

    let conn = db.0.lock().unwrap();
    let sql = format!(
        "SELECT {SUMMARY_COLS}, 1, 0 FROM messages m
         WHERE {}
         ORDER BY m.date DESC
         LIMIT 300",
        conditions.join(" AND ")
    );
    let mut stmt = conn.prepare(&sql)?;
    let refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|a| a.as_ref()).collect();
    let rows = stmt.query_map(refs.as_slice(), row_to_summary)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Kolejkuje wiadomość do wysyłki (Spark: „wyślij później" gdy `send_at`
/// ustawione) i od razu uruchamia przetwarzanie kolejki SMTP.
#[tauri::command]
pub async fn queue_send(app: AppHandle, db: State<'_, Db>, draft: ComposeDraft) -> Result<i64> {
    use base64::Engine;
    let id = {
        let conn = db.0.lock().unwrap();
        conn.execute(
            "INSERT INTO outbox (account_id, to_addrs, cc_addrs, bcc_addrs, subject,
                                 body_text, body_html, send_at, in_reply_to, msg_references)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                draft.account_id,
                draft.to_addrs,
                draft.cc_addrs,
                draft.bcc_addrs,
                draft.subject,
                draft.body_text,
                draft.body_html,
                draft.send_at,
                draft.in_reply_to,
                draft.references
            ],
        )?;
        let id = conn.last_insert_rowid();
        for a in &draft.attachments {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&a.data_b64)
                .map_err(|e| AppError::Other(format!("załącznik „{}”: {e}", a.filename)))?;
            conn.execute(
                "INSERT INTO outbox_attachments (outbox_id, filename, mime, data)
                 VALUES (?1, ?2, ?3, ?4)",
                params![id, a.filename, a.mime, bytes],
            )?;
        }
        id
    };
    tauri::async_runtime::spawn(async move {
        crate::send::process_outbox(&app).await;
    });
    Ok(id)
}

/// Zapisuje kopię roboczą. Bez `id` zakłada nową, z `id` nadpisuje istniejącą.
/// Wołane przy pisaniu (z opóźnieniem), więc musi być tanie: załączniki - a to
/// jedyne, co waży - przepisujemy dopiero, gdy faktycznie się zmieniły.
#[tauri::command]
pub async fn save_draft(db: State<'_, Db>, draft: DraftInput) -> Result<i64> {
    use base64::Engine;
    let conn = db.0.lock().unwrap();
    let id = match draft.id {
        Some(id) => {
            let changed = conn.execute(
                "UPDATE drafts SET account_id = ?2, to_addrs = ?3, cc_addrs = ?4, bcc_addrs = ?5,
                        in_reply_to = ?6, refs = ?7, subject = ?8, body_html = ?9, is_reply = ?10,
                        updated_at = unixepoch()
                 WHERE id = ?1",
                params![
                    id,
                    draft.account_id,
                    draft.to_addrs,
                    draft.cc_addrs,
                    draft.bcc_addrs,
                    draft.in_reply_to,
                    draft.references,
                    draft.subject,
                    draft.body_html,
                    draft.is_reply as i64,
                ],
            )?;
            // Szkic mógł zniknąć (wysłany albo odrzucony w innym miejscu) -
            // wtedy zapis zakłada go na nowo, zamiast przepaść.
            if changed == 0 {
                insert_draft(&conn, &draft)?
            } else {
                id
            }
        }
        None => insert_draft(&conn, &draft)?,
    };

    // Porównanie po nazwie i rozmiarze wystarcza: załączników szkicu nie da się
    // podmienić w miejscu, można je tylko dodać albo usunąć.
    let mut stmt = conn.prepare(
        "SELECT filename, LENGTH(data) FROM draft_attachments WHERE draft_id = ?1 ORDER BY id",
    )?;
    let have: Vec<(String, i64)> = stmt
        .query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);
    let want: Vec<(String, i64)> = draft
        .attachments
        .iter()
        .map(|a| (a.filename.clone(), a.size))
        .collect();
    if have != want {
        conn.execute("DELETE FROM draft_attachments WHERE draft_id = ?1", [id])?;
        for a in &draft.attachments {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&a.data_b64)
                .map_err(|e| AppError::Other(format!("załącznik „{}”: {e}", a.filename)))?;
            conn.execute(
                "INSERT INTO draft_attachments (draft_id, filename, mime, data)
                 VALUES (?1, ?2, ?3, ?4)",
                params![id, a.filename, a.mime, bytes],
            )?;
        }
    }
    Ok(id)
}

fn insert_draft(conn: &rusqlite::Connection, draft: &DraftInput) -> Result<i64> {
    conn.execute(
        "INSERT INTO drafts (account_id, to_addrs, cc_addrs, bcc_addrs, in_reply_to, refs,
                             subject, body_html, is_reply, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, unixepoch())",
        params![
            draft.account_id,
            draft.to_addrs,
            draft.cc_addrs,
            draft.bcc_addrs,
            draft.in_reply_to,
            draft.references,
            draft.subject,
            draft.body_html,
            draft.is_reply as i64,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Wszystkie kopie robocze, od ostatnio ruszanej. Załączniki wracają bez
/// treści - lista rysuje z nich tylko spinacz, a przepisywanie megabajtów do
/// base64 przy każdym zapisie kosztowałoby więcej niż całe pisanie maila.
/// Pełny szkic (z załącznikami) daje `get_draft`.
#[tauri::command]
pub async fn list_drafts(db: State<'_, Db>) -> Result<Vec<StoredDraft>> {
    let conn = db.0.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT id, account_id, to_addrs, cc_addrs, bcc_addrs, in_reply_to, refs,
                subject, body_html, is_reply, updated_at
         FROM drafts ORDER BY updated_at DESC, id DESC",
    )?;
    let mut drafts = stmt
        .query_map([], |r| {
            Ok(StoredDraft {
                id: r.get(0)?,
                account_id: r.get(1)?,
                to_addrs: r.get(2)?,
                cc_addrs: r.get(3)?,
                bcc_addrs: r.get(4)?,
                in_reply_to: r.get(5)?,
                references: r.get(6)?,
                subject: r.get(7)?,
                body_html: r.get(8)?,
                is_reply: r.get::<_, i64>(9)? != 0,
                updated_at: r.get(10)?,
                attachments: Vec::new(),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);

    let mut stmt = conn.prepare(
        "SELECT draft_id, filename, mime, LENGTH(data) FROM draft_attachments ORDER BY id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            DraftAttachment {
                filename: r.get(1)?,
                mime: r.get(2)?,
                size: r.get(3)?,
                data_b64: String::new(),
            },
        ))
    })?;
    for row in rows {
        let (draft_id, attachment) = row?;
        if let Some(d) = drafts.iter_mut().find(|d| d.id == draft_id) {
            d.attachments.push(attachment);
        }
    }
    Ok(drafts)
}

/// Jedna kopia robocza w całości - wołane, gdy szkic wraca do edytora.
#[tauri::command]
pub async fn get_draft(db: State<'_, Db>, id: i64) -> Result<StoredDraft> {
    use base64::Engine;
    let conn = db.0.lock().unwrap();
    let mut draft = conn.query_row(
        "SELECT id, account_id, to_addrs, cc_addrs, bcc_addrs, in_reply_to, refs,
                subject, body_html, is_reply, updated_at
         FROM drafts WHERE id = ?1",
        [id],
        |r| {
            Ok(StoredDraft {
                id: r.get(0)?,
                account_id: r.get(1)?,
                to_addrs: r.get(2)?,
                cc_addrs: r.get(3)?,
                bcc_addrs: r.get(4)?,
                in_reply_to: r.get(5)?,
                references: r.get(6)?,
                subject: r.get(7)?,
                body_html: r.get(8)?,
                is_reply: r.get::<_, i64>(9)? != 0,
                updated_at: r.get(10)?,
                attachments: Vec::new(),
            })
        },
    )?;
    let mut stmt = conn.prepare(
        "SELECT filename, mime, data FROM draft_attachments WHERE draft_id = ?1 ORDER BY id",
    )?;
    let rows = stmt.query_map([id], |r| {
        let data: Vec<u8> = r.get(2)?;
        Ok(DraftAttachment {
            filename: r.get(0)?,
            mime: r.get(1)?,
            size: data.len() as i64,
            data_b64: base64::engine::general_purpose::STANDARD.encode(&data),
        })
    })?;
    draft.attachments = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(draft)
}

/// Kasuje kopię roboczą - po wysyłce albo po odrzuceniu szkicu.
#[tauri::command]
pub async fn delete_draft(db: State<'_, Db>, id: i64) -> Result<()> {
    let conn = db.0.lock().unwrap();
    conn.execute("DELETE FROM drafts WHERE id = ?1", [id])?;
    Ok(())
}

/// Wczytuje plik z dysku jako załącznik szkicu (ścieżka z okna wyboru pliku).
/// Pliki upuszczone na okno idą inną drogą - przeglądarka daje ich zawartość
/// bezpośrednio, bez ścieżki.
#[tauri::command]
pub async fn read_attachment(path: String) -> Result<DraftAttachment> {
    use base64::Engine;
    const LIMIT: u64 = 25 * 1024 * 1024;
    let meta = std::fs::metadata(&path)?;
    if meta.len() > LIMIT {
        return Err(AppError::Other(format!(
            "plik ma {:.1} MB - większość serwerów odrzuca załączniki powyżej 25 MB",
            meta.len() as f64 / 1_048_576.0
        )));
    }
    let bytes = std::fs::read(&path)?;
    let file = std::path::Path::new(&path);
    let filename = file
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "zalacznik".into());
    let ext = file
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    Ok(DraftAttachment {
        filename,
        mime: mime_for(&ext).to_string(),
        size: bytes.len() as i64,
        data_b64: base64::engine::general_purpose::STANDARD.encode(&bytes),
    })
}

/// Typ MIME po rozszerzeniu - tyle, ile potrzeba, żeby odbiorca dostał plik
/// z sensownym typem. Reszta idzie jako strumień bajtów.
fn mime_for(ext: &str) -> &'static str {
    match ext {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "txt" | "log" | "csv" => "text/plain; charset=utf-8",
        "html" | "htm" => "text/html; charset=utf-8",
        "zip" => "application/zip",
        "7z" => "application/x-7z-compressed",
        "rar" => "application/vnd.rar",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "msg" => "application/vnd.ms-outlook",
        "eml" => "message/rfc822",
        _ => "application/octet-stream",
    }
}

/// Ustawia nazwę nadawcy (pole From) dla konta.
#[tauri::command]
pub async fn set_sender_name(db: State<'_, Db>, account_id: i64, name: String) -> Result<()> {
    let conn = db.0.lock().unwrap();
    conn.execute(
        "UPDATE accounts SET sender_name = ?2 WHERE id = ?1",
        params![account_id, name],
    )?;
    Ok(())
}

/// Etykieta konta w panelu bocznym (pusta = pokazujemy adres e-mail).
#[tauri::command]
pub async fn set_account_label(db: State<'_, Db>, account_id: i64, label: String) -> Result<()> {
    let conn = db.0.lock().unwrap();
    conn.execute(
        "UPDATE accounts SET display_name = ?2 WHERE id = ?1",
        params![account_id, label.trim()],
    )?;
    Ok(())
}

/// Podpowiedzi adresatów: nadawcy z historii poczty, najczęstsi i najświeżsi
/// na górze.
#[tauri::command]
pub async fn search_contacts(db: State<'_, Db>, query: String) -> Result<Vec<Contact>> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(vec![]);
    }
    let pattern = format!("%{q}%");
    let conn = db.0.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT from_addr, MAX(from_name) AS name
         FROM messages
         WHERE from_addr != '' AND (from_addr LIKE ?1 OR from_name LIKE ?1)
         GROUP BY lower(from_addr)
         ORDER BY COUNT(*) DESC, MAX(date) DESC
         LIMIT 8",
    )?;
    let rows = stmt.query_map([pattern], |r| {
        Ok(Contact {
            addr: r.get(0)?,
            name: r.get(1)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Załączniki wiadomości (dociągane z serwera przy pierwszym otwarciu).
#[tauri::command]
pub async fn get_attachments(
    app: AppHandle,
    message_id: i64,
) -> Result<Vec<crate::attachments::Attachment>> {
    crate::attachments::get(&app, message_id).await
}

/// Kopiuje załącznik pod ścieżkę wskazaną przez użytkownika.
#[tauri::command]
pub async fn save_attachment(app: AppHandle, attachment_id: i64, target: String) -> Result<()> {
    crate::attachments::save_as(&app, attachment_id, &target)
}

/// Tworzy folder na serwerze IMAP i zapisuje go lokalnie.
#[tauri::command]
pub async fn create_folder(app: AppHandle, account_id: i64, name: String) -> Result<i64> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::Other("nazwa folderu nie może być pusta".into()));
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
    // Nazwy z polskimi znakami serwer przyjmuje w zmodyfikowanym UTF-7.
    let raw = utf7_imap::encode_utf7_imap(name.clone());

    let mut session = crate::sync::connect_session(&host, port, &login, &password).await?;
    let created = session.create(&raw).await;
    session.logout().await.ok();
    if let Err(e) = created {
        // Folder mógł już istnieć na serwerze - wtedy tylko dopisujemy lokalnie.
        let msg = e.to_string();
        if !msg.to_lowercase().contains("alreadyexists") && !msg.contains("[ALREADYEXISTS]") {
            return Err(AppError::Other(format!("serwer odrzucił utworzenie folderu: {msg}")));
        }
    }

    let db = app.state::<Db>();
    let conn = db.0.lock().unwrap();
    conn.execute(
        "INSERT INTO folders (account_id, name, display_name, kind) VALUES (?1, ?2, ?3, 'custom')
         ON CONFLICT(account_id, name) DO UPDATE SET display_name = excluded.display_name",
        params![account_id, raw, name],
    )?;
    Ok(conn.query_row(
        "SELECT id FROM folders WHERE account_id = ?1 AND name = ?2",
        params![account_id, raw],
        |r| r.get(0),
    )?)
}

/// Usuwa folder z serwera IMAP i lokalnie (wraz z wiadomościami).
#[tauri::command]
pub async fn delete_folder(app: AppHandle, folder_id: i64) -> Result<()> {
    let (account_id, raw_name, kind) = {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        conn.query_row(
            "SELECT account_id, name, kind FROM folders WHERE id = ?1",
            [folder_id],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )?
    };
    if kind == "inbox" {
        return Err(AppError::Other("skrzynki odbiorczej nie można usunąć".into()));
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
    // IMAP nie pozwala usunąć folderu z podfolderami - kasujemy całą gałąź,
    // zaczynając od najgłębszego (np. Trash.Mailspring.Snoozed przed Mailspring).
    let mut branch: Vec<(i64, String)> = {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name FROM folders
             WHERE account_id = ?1 AND (name LIKE ?2 OR name LIKE ?3)",
        )?;
        let rows = stmt.query_map(
            params![account_id, format!("{raw_name}.%"), format!("{raw_name}/%")],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    branch.sort_by_key(|(_, name)| std::cmp::Reverse(name.len()));
    branch.push((folder_id, raw_name));

    let login = if login.is_empty() { email.clone() } else { login };
    let password = crate::accounts::get_password(&email)?;
    let mut session = crate::sync::connect_session(&host, port, &login, &password).await?;
    let mut failure = None;
    let mut removed = Vec::new();
    for (id, name) in &branch {
        match session.delete(name).await {
            Ok(()) => removed.push(*id),
            Err(e) => {
                // Folderu już nie ma na serwerze (np. skasowany w innym
                // programie) - u nas też ma zniknąć, to nie jest błąd.
                let msg = e.to_string().to_lowercase();
                if msg.contains("nonexistent")
                    || msg.contains("does not exist")
                    || msg.contains("doesn't exist")
                {
                    removed.push(*id);
                    continue;
                }
                failure = Some(format!("serwer odrzucił usunięcie folderu {name}: {e}"));
                break;
            }
        }
    }
    session.logout().await.ok();

    {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        for id in removed {
            conn.execute("DELETE FROM folders WHERE id = ?1", [id])?;
        }
    }
    match failure {
        Some(msg) => Err(AppError::Other(msg)),
        None => Ok(()),
    }
}

#[tauri::command]
pub async fn list_rules(db: State<'_, Db>) -> Result<Vec<Rule>> {
    let conn = db.0.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT r.id, r.account_id,
                json_extract(r.conditions_json, '$.from'),
                json_extract(r.actions_json, '$.moveTo'),
                COALESCE(f.display_name, '(usunięty folder)'), r.enabled
         FROM rules r
         LEFT JOIN folders f ON f.id = json_extract(r.actions_json, '$.moveTo')
         ORDER BY r.position, r.id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(Rule {
            id: r.get(0)?,
            account_id: r.get(1)?,
            from_addr: r.get(2)?,
            folder_id: r.get(3)?,
            folder_name: r.get(4)?,
            enabled: r.get::<_, i64>(5)? != 0,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Zapisuje regułę „od tego nadawcy do tego folderu". Przeniesienie wykona
/// synchronizacja (także dla wiadomości już pobranych).
#[tauri::command]
pub async fn add_rule(
    app: AppHandle,
    account_id: i64,
    from_addr: String,
    folder_id: i64,
) -> Result<()> {
    {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        let folder_name: String =
            conn.query_row("SELECT display_name FROM folders WHERE id = ?1", [folder_id], |r| {
                r.get(0)
            })?;
        // Ta sama reguła zapisana ponownie nie tworzy duplikatu.
        conn.execute(
            "INSERT OR IGNORE INTO rules (account_id, name, conditions_json, actions_json)
             VALUES (?1, ?2, json_object('from', ?3), json_object('moveTo', ?4))",
            params![
                account_id,
                format!("Od {from_addr} do {folder_name}"),
                from_addr.to_lowercase(),
                folder_id
            ],
        )?;
    }
    // Reguła obejmuje też wiadomości już w skrzynce - synchronizujemy od razu.
    tauri::async_runtime::spawn(async move {
        if let Err(e) = crate::sync::sync_account(&app, account_id).await {
            eprintln!("[rules] synchronizacja po dodaniu reguły: {e}");
        }
    });
    Ok(())
}

#[tauri::command]
pub async fn delete_rule(db: State<'_, Db>, id: i64) -> Result<()> {
    let conn = db.0.lock().unwrap();
    conn.execute("DELETE FROM rules WHERE id = ?1", [id])?;
    Ok(())
}

/// Podpisy z klasycznego Outlooka (%APPDATA%\Microsoft\Signatures).
#[tauri::command]
pub async fn list_outlook_signatures() -> Result<Vec<crate::outlook::OutlookSignature>> {
    crate::outlook::list()
}

#[tauri::command]
pub async fn get_setting(db: State<'_, Db>, key: String) -> Result<Option<String>> {
    let conn = db.0.lock().unwrap();
    match conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0)) {
        Ok(v) => Ok(Some(v)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

#[tauri::command]
pub async fn set_setting(db: State<'_, Db>, key: String, value: String) -> Result<()> {
    let conn = db.0.lock().unwrap();
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

#[tauri::command]
pub async fn sync_now(app: AppHandle, account_id: i64) -> Result<()> {
    sync::sync_account(&app, account_id).await
}

/// Ręczne „sprawdź pocztę": szybkie zajrzenie do skrzynek odbiorczych.
#[tauri::command]
pub async fn check_mail(app: AppHandle) -> Result<()> {
    sync::sync_all_mode(&app, sync::SyncMode::Quick).await;
    Ok(())
}

/// Wykrywa ustawienia serwerów pocztowych dla adresu (znani dostawcy →
/// autokonfiguracja Thunderbirda → zgadywanie hostów).
#[tauri::command]
pub async fn detect_settings(email: String) -> Result<crate::detect::DetectedConfig> {
    Ok(crate::detect::detect(&email).await)
}

/// Próbuje zalogować się do IMAP podanymi poświadczeniami, niczego nie zapisując.
#[tauri::command]
pub async fn test_login(host: String, port: u16, login: String, password: String) -> Result<()> {
    sync::try_login(&host, port, &login, &password).await
}

/// Wypełnia bazę przykładowymi danymi, żeby obejrzeć interfejs bez
/// skonfigurowanego konta. Nic nie robi, jeśli jakiekolwiek konto istnieje.
#[tauri::command]
pub async fn seed_demo_data(db: State<'_, Db>) -> Result<bool> {
    let conn = db.0.lock().unwrap();
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM accounts", [], |r| r.get(0))?;
    if count > 0 {
        return Ok(false);
    }
    conn.execute_batch(include_str!("demo_seed.sql"))?;
    Ok(true)
}

/// Szuka kandydatów do sprzątania: nadawców, od których nazbierało się dużo
/// newsletterów i powiadomień.
///
/// Świadomie pomijamy Wysłane, Wersje robocze i Kosz - własnej korespondencji
/// się nie sprząta, a pomyłka bolałaby tam najbardziej. Bierzemy wyłącznie
/// kategorie `newsletters` i `notifications`, więc zwykła poczta od ludzi
/// nie trafi na listę, choćby było jej najwięcej.
#[tauri::command]
pub async fn cleanup_scan(
    db: State<'_, Db>,
    account_id: Option<i64>,
    min_count: i64,
    older_than_days: Option<i64>,
    only_unread: bool,
) -> Result<Vec<CleanupGroup>> {
    let cutoff = older_than_days
        .map(|d| chrono::Utc::now().timestamp() - d * 86_400)
        .unwrap_or(i64::MAX);

    let conn = db.0.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT m.id, m.from_addr, m.from_name, m.date, m.is_read, m.category, m.subject
         FROM messages m JOIN folders f ON f.id = m.folder_id
         WHERE f.kind NOT IN ('sent', 'drafts', 'trash')
           AND m.category IN ('newsletters', 'notifications')
           AND m.snoozed_until IS NULL
           AND (?1 IS NULL OR f.account_id = ?1)
           AND (?2 = 0 OR m.date < ?3)
           AND (?4 = 0 OR m.is_read = 0)
         ORDER BY m.date DESC",
    )?;
    let rows = stmt.query_map(
        params![
            account_id,
            older_than_days.is_some() as i64,
            cutoff,
            only_unread as i64
        ],
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, bool>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
            ))
        },
    )?;

    // Grupujemy po adresie. Wiersze przychodzą od najnowszego, więc pierwsze
    // trafienia dają zarówno nazwę nadawcy, jak i tematy na podgląd.
    let mut groups: std::collections::HashMap<String, CleanupGroup> =
        std::collections::HashMap::new();
    for row in rows.flatten() {
        let (id, from_addr, from_name, date, is_read, category, subject) = row;
        let key = from_addr.to_lowercase();
        let group = groups.entry(key).or_insert_with(|| CleanupGroup {
            from_addr,
            from_name,
            count: 0,
            unread: 0,
            never_read: 0,
            oldest: date,
            newest: date,
            category,
            samples: Vec::new(),
            ids: Vec::new(),
        });
        group.count += 1;
        if !is_read {
            group.unread += 1;
            group.never_read += 1;
        }
        group.oldest = group.oldest.min(date);
        group.newest = group.newest.max(date);
        if group.samples.len() < 3 && !subject.trim().is_empty() {
            group.samples.push(subject);
        }
        group.ids.push(id);
    }

    let mut out: Vec<CleanupGroup> = groups
        .into_values()
        .filter(|g| g.count >= min_count.max(1))
        .collect();
    // Najpierw najgrubsze zbiory - tam sprzątanie daje najwięcej.
    out.sort_by(|a, b| b.count.cmp(&a.count));
    Ok(out)
}

/// Plan kasowania dla jednego folderu: co wysłać na serwer i co wyczyścić lokalnie.
struct FolderPurge {
    folder_name: String,
    folder_kind: String,
    uids: Vec<u32>,
    ids: Vec<i64>,
}

/// Kasuje wskazane wiadomości hurtem. W odróżnieniu od `delete_message`
/// otwiera jedno połączenie na konto i jeden SELECT na folder, a UID-y wysyła
/// paczkami - przy kilku tysiącach maili osobne sesje trwałyby godzinami.
///
/// Wiadomości trafiają do Kosza (IMAP MOVE), a nie znikają nieodwracalnie;
/// dopiero z samego Kosza kasujemy na twardo, tak jak przy pojedynczym mailu.
#[tauri::command]
pub async fn cleanup_delete(app: AppHandle, ids: Vec<i64>) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    // Plan: konto -> folder -> UID-y. Budujemy go jednym przejściem po bazie,
    // żeby nie trzymać mutexa przez czas rozmowy z serwerem.
    let mut plan: std::collections::HashMap<i64, std::collections::HashMap<i64, FolderPurge>> =
        std::collections::HashMap::new();
    {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT m.id, m.uid, f.id, f.name, f.kind, f.account_id
             FROM messages m JOIN folders f ON f.id = m.folder_id
             WHERE m.id = ?1",
        )?;
        for id in &ids {
            let row = stmt.query_row([id], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, u32>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, i64>(5)?,
                ))
            });
            let Ok((id, uid, folder_id, folder_name, folder_kind, account_id)) = row else {
                continue;
            };
            let entry = plan
                .entry(account_id)
                .or_default()
                .entry(folder_id)
                .or_insert_with(|| FolderPurge {
                    folder_name,
                    folder_kind,
                    uids: Vec::new(),
                    ids: Vec::new(),
                });
            if uid > 0 {
                entry.uids.push(uid);
            }
            entry.ids.push(id);
        }
    }

    let total: usize = plan
        .values()
        .flat_map(|f| f.values())
        .map(|p| p.ids.len())
        .sum();
    let mut deleted = 0usize;
    // Liczone po stronie serwera - to ono trwa, więc na tym opiera się pasek.
    let mut moved = 0usize;

    for (account_id, folders) in plan {
        let (email, login, host, port, trash) = {
            let db = app.state::<Db>();
            let conn = db.0.lock().unwrap();
            let account = conn.query_row(
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
            let trash: Option<String> = conn
                .query_row(
                    "SELECT name FROM folders WHERE account_id = ?1 AND kind = 'trash' LIMIT 1",
                    [account_id],
                    |r| r.get(0),
                )
                .ok();
            (account.0, account.1, account.2, account.3, trash)
        };
        let login = if login.is_empty() { email.clone() } else { login };
        let password = crate::accounts::get_password(&email)?;
        let mut session = crate::sync::connect_session(&host, port, &login, &password).await?;

        for folder in folders.into_values() {
            if folder.uids.is_empty() {
                // Wiadomość bez UID-a żyje tylko lokalnie (dane demo).
                deleted += purge_local(&app, &folder.ids)?;
                continue;
            }
            session.select(&folder.folder_name).await?;
            // Paczkami, bo linia polecenia IMAP ma ograniczoną długość.
            for chunk in folder.uids.chunks(200) {
                let uid_set = chunk
                    .iter()
                    .map(|u| u.to_string())
                    .collect::<Vec<_>>()
                    .join(",");
                let result = match &trash {
                    Some(target) if folder.folder_kind != "trash" => {
                        session.uid_mv(&uid_set, target).await
                    }
                    _ => {
                        let stream = session
                            .uid_store(&uid_set, "+FLAGS.SILENT (\\Deleted)")
                            .await?;
                        stream.try_collect::<Vec<_>>().await?;
                        match session.expunge().await {
                    Ok(stream) => stream.try_collect::<Vec<_>>().await.map(|_| ()),
                    Err(e) => Err(e),
                }
                    }
                };
                result.map_err(|e| AppError::Other(format!("serwer odrzucił usunięcie: {e}")))?;
                // Postęp po każdej paczce, a nie po całym folderze: przy
                // kilku tysiącach maili jeden folder to minuty ciszy.
                moved += chunk.len();
                let _ = app.emit("cleanup-progress", CleanupProgress { done: moved, total });
                let _ = app.emit("sync-status", format!("Sprzątanie: {moved} z {total}"));
            }
            deleted += purge_local(&app, &folder.ids)?;
        }
        session.logout().await.ok();
    }

    let _ = app.emit("sync-status", "");
    let _ = app.emit("messages-updated", ());
    Ok(deleted)
}

/// Usuwa wiadomości z lokalnej bazy po udanym skasowaniu na serwerze.
fn purge_local(app: &AppHandle, ids: &[i64]) -> Result<usize> {
    let db = app.state::<Db>();
    let conn = db.0.lock().unwrap();
    let mut removed = 0usize;
    for id in ids {
        removed += conn.execute("DELETE FROM messages WHERE id = ?1", [id])?;
    }
    Ok(removed)
}

/// Opróżnia Kosz do zera - na serwerze i lokalnie.
///
/// W odróżnieniu od sprzątania nie ma tu dokąd przenosić: zawartość Kosza
/// kasujemy nieodwracalnie (`\Deleted` na całym zakresie UID + EXPUNGE).
/// Bierzemy `1:*`, a nie identyfikatory z bazy, żeby zniknęło też to, czego
/// LotusMail nigdy nie zdążył pobrać.
#[tauri::command]
pub async fn empty_trash(app: AppHandle, folder_id: i64) -> Result<usize> {
    let (folder_name, folder_kind, account_id) = {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        conn.query_row(
            "SELECT name, kind, account_id FROM folders WHERE id = ?1",
            [folder_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            },
        )?
    };
    if folder_kind != "trash" {
        return Err(AppError::Other("to nie jest folder Kosz".into()));
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

    let _ = app.emit("sync-status", "Opróżniam Kosz…");
    let mut session = crate::sync::connect_session(&host, port, &login, &password).await?;
    session.select(&folder_name).await?;

    // Jawna lista UID-ów zamiast zakresu `1:*`. Gmail potrafi przyjąć
    // `\Deleted` na całości i zignorować gołe EXPUNGE - wtedy Kosz zostaje
    // pełen samych oznaczonych wiadomości. Przy jawnej liście da się też
    // usuwać paczkami i meldować postęp.
    let mut uids: Vec<u32> = session.uid_search("ALL").await?.into_iter().collect();
    uids.sort_unstable();
    let total = uids.len();

    // UID EXPUNGE (RFC 4315) usuwa dokładnie wskazane wiadomości. Serwer bez
    // UIDPLUS dostaje na koniec gołe EXPUNGE - jeden przebieg na całość.
    let uidplus = session
        .capabilities()
        .await
        .map(|c| c.has_str("UIDPLUS"))
        .unwrap_or(false);

    let mut processed = 0usize;
    for chunk in uids.chunks(500) {
        let uid_set = chunk
            .iter()
            .map(|u| u.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let stream = session.uid_store(&uid_set, "+FLAGS.SILENT (\\Deleted)").await?;
        stream.try_collect::<Vec<_>>().await?;
        if uidplus {
            let stream = session
                .uid_expunge(&uid_set)
                .await
                .map_err(|e| AppError::Other(format!("serwer odrzucił opróżnienie Kosza: {e}")))?;
            stream
                .try_collect::<Vec<_>>()
                .await
                .map_err(|e| AppError::Other(format!("serwer przerwał opróżnianie Kosza: {e}")))?;
        }
        processed += chunk.len();
        let _ = app.emit(
            "cleanup-progress",
            CleanupProgress {
                done: processed,
                total,
            },
        );
        let _ = app.emit("sync-status", format!("Opróżniam Kosz: {processed} z {total}"));
    }
    if !uidplus && total > 0 {
        let stream = session
            .expunge()
            .await
            .map_err(|e| AppError::Other(format!("serwer odrzucił opróżnienie Kosza: {e}")))?;
        stream
            .try_collect::<Vec<_>>()
            .await
            .map_err(|e| AppError::Other(format!("serwer przerwał opróżnianie Kosza: {e}")))?;
    }

    // Sprawdzamy, co naprawdę zostało - serwer bywa uparty, a cicha porażka
    // byłaby gorsza niż komunikat.
    let left = session.select(&folder_name).await.map(|m| m.exists).unwrap_or(0);
    session.logout().await.ok();

    {
        let db = app.state::<Db>();
        let conn = db.0.lock().unwrap();
        conn.execute("DELETE FROM messages WHERE folder_id = ?1", [folder_id])?;
        // Licznik UID-ów zostaje tam, gdzie był, a folder jest domknięty.
        //
        // Kuszące jest wyzerowanie go „bo przecież nic tam nie ma", ale skutek
        // jest odwrotny do zamierzonego: tania ścieżka pyta wtedy `UID 1:*`,
        // czyli o całą zawartość folderu, i przy każdym przebiegu próbuje
        // pobrać tysiące wiadomości z Kosza. Pytanie o UID-y, które już nie
        // istnieją, jest niegroźne - serwer po prostu nic nie zwraca.
        conn.execute(
            "UPDATE folders SET backfilled = 1 WHERE id = ?1",
            [folder_id],
        )?;
    }

    let _ = app.emit("sync-status", "");
    let _ = app.emit("messages-updated", ());
    if left > 0 {
        return Err(AppError::Other(format!(
            "serwer zostawił {left} wiadomości w Koszu mimo polecenia usunięcia"
        )));
    }
    Ok(total)
}

/// Hasło synchronizacji trzymamy w pęku kluczy, nigdy w bazie - to ono chroni
/// paczkę leżącą na cudzym serwerze, więc obowiązuje je ta sama zasada
/// co hasła do kont.
const SYNC_KEY_ACCOUNT: &str = "__lotusmail_sync__";

#[tauri::command]
pub async fn sync_set_passphrase(passphrase: String) -> Result<()> {
    if passphrase.trim().is_empty() {
        crate::accounts::delete_password(SYNC_KEY_ACCOUNT)?;
        return Ok(());
    }
    crate::accounts::store_password(SYNC_KEY_ACCOUNT, &passphrase)
}

/// Czy hasło synchronizacji jest już ustawione. Samego hasła nie oddajemy
/// interfejsowi - nie ma po co opuszczać rdzenia.
#[tauri::command]
pub async fn sync_has_passphrase() -> Result<bool> {
    Ok(crate::accounts::get_password(SYNC_KEY_ACCOUNT).is_ok())
}

fn sync_passphrase() -> Result<String> {
    crate::accounts::get_password(SYNC_KEY_ACCOUNT).map_err(|_| {
        AppError::Other("nie ustawiono hasła synchronizacji".into())
    })
}

/// Kod do przeniesienia ręcznego - do skopiowania i wklejenia na drugim
/// urządzeniu, gdy nie chce się używać nośnika w skrzynce.
#[tauri::command]
pub async fn sync_export(app: AppHandle) -> Result<String> {
    let passphrase = sync_passphrase()?;
    let payload = crate::sync_config::collect(&app)?;
    crate::sync_config::seal(&payload, &passphrase)
}

#[tauri::command]
pub async fn sync_import(app: AppHandle, blob: String) -> Result<crate::sync_config::ApplyResult> {
    let passphrase = sync_passphrase()?;
    let payload = crate::sync_config::open(&blob, &passphrase)?;
    let result = crate::sync_config::apply(&app, &payload)?;
    if result.added > 0 {
        // Nowe konta nie mają jeszcze folderów ani poczty - niech się pobiorą
        // od razu, bez czekania na cykl harmonogramu.
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            crate::sync::sync_all(&handle).await;
        });
    }
    Ok(result)
}

/// Odkłada konfigurację w folderze `LotusMail` na wskazanym koncie.
#[tauri::command]
pub async fn sync_push(app: AppHandle, account_id: i64) -> Result<usize> {
    let passphrase = sync_passphrase()?;
    let _ = app.emit("sync-status", "Zapisuję konfigurację w skrzynce…");
    let out = crate::sync_config::push(&app, account_id, &passphrase).await;
    let _ = app.emit("sync-status", "");
    out
}

/// Pobiera konfigurację z folderu `LotusMail` i wgrywa ją tutaj.
#[tauri::command]
pub async fn sync_pull(app: AppHandle, account_id: i64) -> Result<crate::sync_config::ApplyResult> {
    let passphrase = sync_passphrase()?;
    let _ = app.emit("sync-status", "Pobieram konfigurację ze skrzynki…");
    let result = crate::sync_config::pull(&app, account_id, &passphrase).await;
    let _ = app.emit("sync-status", "");
    let result = result?;
    if result.added > 0 {
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            crate::sync::sync_all(&handle).await;
        });
    }
    Ok(result)
}

/// Zapisuje kolejność folderów ustawioną przeciąganiem w panelu bocznym.
///
/// Dostajemy pełną listę folderów konta w docelowej kolejności i numerujemy je
/// od jedynki. Zero zostaje zarezerwowane dla folderów nigdy nieprzestawianych,
/// które mają sortować się po dawnemu - alfabetycznie, ze skrzynką odbiorczą
/// na czele.
#[tauri::command]
pub async fn reorder_folders(db: State<'_, Db>, folder_ids: Vec<i64>) -> Result<()> {
    let conn = db.0.lock().unwrap();
    let tx = conn.unchecked_transaction()?;
    for (i, id) in folder_ids.iter().enumerate() {
        tx.execute(
            "UPDATE folders SET sort_order = ?2 WHERE id = ?1",
            rusqlite::params![id, (i + 1) as i64],
        )?;
    }
    tx.commit()?;
    Ok(())
}
