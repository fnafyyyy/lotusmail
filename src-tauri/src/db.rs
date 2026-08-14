use crate::error::Result;
use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

/// Stan aplikacji: jedno połączenie SQLite za mutexem.
/// Zapytania są krótkie (odczyt z lokalnej bazy), więc prosty mutex wystarcza.
pub struct Db(pub Mutex<Connection>);

pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> Result<()> {
    let version: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version < 1 {
        conn.execute_batch(SCHEMA_V1)?;
        conn.pragma_update(None, "user_version", 1)?;
    }
    if version < 2 {
        // Login do serwera bywa inny niż adres (np. WP loguje częścią lokalną).
        conn.execute_batch("ALTER TABLE accounts ADD COLUMN login TEXT NOT NULL DEFAULT ''")?;
        conn.pragma_update(None, "user_version", 2)?;
    }
    if version < 3 {
        // Ustawienia aplikacji (stopka itd.) - proste klucz/wartość.
        conn.execute_batch("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)")?;
        conn.pragma_update(None, "user_version", 3)?;
    }
    if version < 4 {
        // Nazwa nadawcy w polu From - co innego niż etykieta konta w sidebarze.
        conn.execute_batch("ALTER TABLE accounts ADD COLUMN sender_name TEXT NOT NULL DEFAULT ''")?;
        conn.pragma_update(None, "user_version", 4)?;
    }
    if version < 5 {
        // Wątkowanie konwersacji: nagłówki In-Reply-To/References oraz klucz
        // tematu (temat bez przedrostków Re:/Fwd:) dla maili bez tych nagłówków.
        conn.execute_batch(
            "ALTER TABLE messages ADD COLUMN in_reply_to TEXT;
             ALTER TABLE messages ADD COLUMN subject_key TEXT NOT NULL DEFAULT '';
             CREATE INDEX IF NOT EXISTS idx_messages_thread ON messages(thread_id);
             CREATE INDEX IF NOT EXISTS idx_messages_subject_key ON messages(subject_key);",
        )?;
        backfill_threads(conn)?;
        conn.pragma_update(None, "user_version", 5)?;
    }
    if version < 6 {
        // Zmiany flag zrobione lokalnie czekają tu na wysłanie na serwer IMAP.
        conn.execute_batch(
            "ALTER TABLE messages ADD COLUMN flags_dirty INTEGER NOT NULL DEFAULT 0;
             CREATE INDEX IF NOT EXISTS idx_messages_dirty ON messages(flags_dirty)
                 WHERE flags_dirty = 1;",
        )?;
        conn.pragma_update(None, "user_version", 6)?;
    }
    if version < 7 {
        // Wyszukiwanie obejmuje teraz treść wiadomości, nie tylko nagłówki.
        conn.execute_batch(
            "CREATE VIRTUAL TABLE bodies_fts USING fts5(
                 text, content='message_bodies', content_rowid='message_id');

             CREATE TRIGGER bodies_ai AFTER INSERT ON message_bodies BEGIN
                 INSERT INTO bodies_fts(rowid, text) VALUES (new.message_id, COALESCE(new.text, ''));
             END;
             CREATE TRIGGER bodies_ad AFTER DELETE ON message_bodies BEGIN
                 INSERT INTO bodies_fts(bodies_fts, rowid, text)
                 VALUES ('delete', old.message_id, COALESCE(old.text, ''));
             END;
             CREATE TRIGGER bodies_au AFTER UPDATE ON message_bodies BEGIN
                 INSERT INTO bodies_fts(bodies_fts, rowid, text)
                 VALUES ('delete', old.message_id, COALESCE(old.text, ''));
                 INSERT INTO bodies_fts(rowid, text) VALUES (new.message_id, COALESCE(new.text, ''));
             END;

             INSERT INTO bodies_fts(rowid, text)
             SELECT message_id, COALESCE(text, '') FROM message_bodies;",
        )?;
        conn.pragma_update(None, "user_version", 7)?;
    }
    if version < 8 {
        // Załączniki: metadane w bazie, pliki na dysku (obok bazy).
        conn.execute_batch(
            "CREATE TABLE attachments (
                 id         INTEGER PRIMARY KEY,
                 message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
                 name       TEXT NOT NULL,
                 mime       TEXT NOT NULL DEFAULT 'application/octet-stream',
                 size       INTEGER NOT NULL DEFAULT 0,
                 path       TEXT NOT NULL,
                 is_inline  INTEGER NOT NULL DEFAULT 0
             );
             CREATE INDEX idx_attachments_message ON attachments(message_id);",
        )?;
        conn.pragma_update(None, "user_version", 8)?;
    }
    if version < 9 {
        // Reguły zapisywane wielokrotnie tym samym kliknięciem - zostawiamy
        // po jednej z każdej pary (nadawca, folder) i blokujemy duplikaty.
        conn.execute_batch(
            "DELETE FROM rules WHERE id NOT IN (
                 SELECT MIN(id) FROM rules
                 GROUP BY account_id, conditions_json, actions_json
             );
             CREATE UNIQUE INDEX IF NOT EXISTS idx_rules_unique
                 ON rules(account_id, conditions_json, actions_json);",
        )?;
        conn.pragma_update(None, "user_version", 9)?;
    }
    if version < 10 {
        // Indeksy pod widok listy: grupowanie wątków, liczniki nieprzeczytanych
        // i filtr kategorii. Bez nich przełączanie folderów zwalnia z każdą
        // kolejną tysiącką wiadomości.
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_messages_folder_unread
                 ON messages(folder_id, is_read);
             CREATE INDEX IF NOT EXISTS idx_messages_category_date
                 ON messages(category, date DESC);
             CREATE INDEX IF NOT EXISTS idx_messages_thread_date
                 ON messages(thread_id, date DESC);
             ANALYZE;",
        )?;
        conn.pragma_update(None, "user_version", 10)?;
    }
    if version < 11 {
        // Przeklasyfikowanie już pobranych wiadomości nowymi regułami. Nagłówków
        // dla nich nie mamy, więc kierujemy się samym adresem nadawcy - to
        // wystarcza, żeby raporty (backup@, jira@, root@) opuściły „Główne".
        let rows: Vec<(i64, String, String)> = {
            let mut stmt = conn.prepare("SELECT id, from_addr, category FROM messages")?;
            let mapped = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (id, from_addr, current) in rows {
            // Newsletterów nie ruszamy - tam zadziałał nagłówek „wypisz się".
            if current != "primary" {
                continue;
            }
            let nowa = crate::mail::categorize(&from_addr, crate::mail::MailSignals::default());
            if nowa != current {
                conn.execute(
                    "UPDATE messages SET category = ?2 WHERE id = ?1",
                    rusqlite::params![id, nowa],
                )?;
            }
        }
        conn.pragma_update(None, "user_version", 11)?;
    }
    if version < 12 {
        // Znacznik „ten folder mamy w całości". Dzięki niemu pełna
        // synchronizacja nie przemiela co kilka minut całej historii - kompletne
        // foldery sprawdzamy tanio (liczba wiadomości vs EXISTS z serwera),
        // a listę UID-ów pobieramy tylko wtedy, gdy coś się nie zgadza.
        conn.execute_batch("ALTER TABLE folders ADD COLUMN backfilled INTEGER NOT NULL DEFAULT 0")?;
        conn.pragma_update(None, "user_version", 12)?;
    }
    if version < 13 {
        // Załączniki wychodzące trzymamy w bazie razem z kolejką - dzięki temu
        // „wyślij później" przetrwa zamknięcie programu, nawet jeśli plik
        // źródłowy zdąży zniknąć z dysku.
        conn.execute_batch(
            "CREATE TABLE outbox_attachments (
                 id        INTEGER PRIMARY KEY,
                 outbox_id INTEGER NOT NULL REFERENCES outbox(id) ON DELETE CASCADE,
                 filename  TEXT NOT NULL,
                 mime      TEXT NOT NULL DEFAULT 'application/octet-stream',
                 data      BLOB NOT NULL
             );
             CREATE INDEX idx_outbox_attachments ON outbox_attachments(outbox_id);",
        )?;
        conn.pragma_update(None, "user_version", 13)?;
    }
    if version < 14 {
        // Kopie (DW/UDW) oraz nagłówki wątkowania odpowiedzi. `references`
        // jest słowem kluczowym SQL, stąd kolumna `msg_references`.
        conn.execute_batch(
            "ALTER TABLE outbox ADD COLUMN cc_addrs TEXT NOT NULL DEFAULT '';
             ALTER TABLE outbox ADD COLUMN bcc_addrs TEXT NOT NULL DEFAULT '';
             ALTER TABLE outbox ADD COLUMN in_reply_to TEXT;
             ALTER TABLE outbox ADD COLUMN msg_references TEXT;",
        )?;
        conn.pragma_update(None, "user_version", 14)?;
    }
    if version < 15 {
        // Rozplątanie wątków sklejonych po samym temacie - patrz opis funkcji.
        rethread_subject_threads(conn)?;
        conn.pragma_update(None, "user_version", 15)?;
    }
    if version < 16 {
        // Własna kolejność folderów w panelu bocznym. Zero oznacza „nie
        // ustawiono" - wtedy obowiązuje dawne sortowanie alfabetyczne,
        // więc nikomu nic się nie przestawia bez jego udziału.
        conn.execute_batch(
            "ALTER TABLE folders ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0",
        )?;
        conn.pragma_update(None, "user_version", 16)?;
    }
    Ok(())
}

/// Przelicza wątki sklejone po samym temacie.
///
/// Dawna reguła doklejała każdą wiadomość do poprzedniej o tym samym tytule,
/// więc łańcuch maili „test" rósł miesiącami i mieszał korespondencję z różnymi
/// osobami. Nowa reguła pozwala dołączyć po temacie wyłącznie odpowiedziom,
/// ale to samo trzeba zrobić z tym, co już leży w bazie.
///
/// Idziemy po każdej grupie (konto, klucz tematu) w kolejności dat: wiadomość
/// bez przedrostka „Re:"/„Odp:" zaczyna nowy wątek, odpowiedź dokleja się do
/// bieżącego. Dzięki temu prawdziwe rozmowy zostają w całości, a przypadkowe
/// zbieżności tytułów się rozpadają. Wątki wyznaczone z nagłówków
/// References/In-Reply-To zostawiamy nietknięte - tamte są pewne.
fn rethread_subject_threads(conn: &Connection) -> Result<()> {
    let rows: Vec<(i64, String, i64, String, i64)> = {
        let mut stmt = conn.prepare(
            "SELECT f.account_id, m.subject_key, m.id, m.subject, m.date
             FROM messages m JOIN folders f ON f.id = m.folder_id
             WHERE m.thread_id LIKE 's:%' AND m.subject_key <> ''
             ORDER BY f.account_id, m.subject_key, m.date, m.id",
        )?;
        let mapped = stmt.query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?;
        mapped.collect::<rusqlite::Result<Vec<_>>>()?
    };

    let tx = conn.unchecked_transaction()?;
    let mut group: Option<(i64, String)> = None;
    let mut thread = String::new();
    for (account_id, subject_key, id, subject, date) in rows {
        let here = (account_id, subject_key.clone());
        let same_group = group.as_ref() == Some(&here);
        if !same_group || !crate::mail::is_reply_subject(&subject) {
            thread = format!("s:{account_id}:{subject_key}:{date}:{id}");
            group = Some(here);
        }
        tx.execute(
            "UPDATE messages SET thread_id = ?2 WHERE id = ?1",
            rusqlite::params![id, thread],
        )?;
    }
    tx.commit()?;
    Ok(())
}


/// Uzupełnia wątki dla wiadomości pobranych przed wprowadzeniem konwersacji -
/// po kluczu tematu, bo nagłówków References dla nich nie mamy.
fn backfill_threads(conn: &Connection) -> Result<()> {
    let rows: Vec<(i64, String, Option<String>)> = {
        let mut stmt = conn.prepare("SELECT id, subject, message_id FROM messages")?;
        let mapped = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        mapped.collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (id, subject, message_id) in rows {
        let key = crate::mail::subject_key(&subject);
        let thread = if key.is_empty() {
            format!("m:{}", message_id.unwrap_or_else(|| id.to_string()))
        } else {
            format!("s:{key}")
        };
        conn.execute(
            "UPDATE messages SET subject_key = ?2, thread_id = ?3 WHERE id = ?1",
            rusqlite::params![id, key, thread],
        )?;
    }
    Ok(())
}

const SCHEMA_V1: &str = r#"
CREATE TABLE accounts (
    id            INTEGER PRIMARY KEY,
    email         TEXT NOT NULL UNIQUE,
    display_name  TEXT NOT NULL DEFAULT '',
    imap_host     TEXT NOT NULL DEFAULT '',
    imap_port     INTEGER NOT NULL DEFAULT 993,
    smtp_host     TEXT NOT NULL DEFAULT '',
    smtp_port     INTEGER NOT NULL DEFAULT 587,
    auth_kind     TEXT NOT NULL DEFAULT 'password',
    created_at    INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE TABLE folders (
    id            INTEGER PRIMARY KEY,
    account_id    INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    display_name  TEXT NOT NULL,
    kind          TEXT NOT NULL DEFAULT 'custom',
    uid_validity  INTEGER,
    last_seen_uid INTEGER NOT NULL DEFAULT 0,
    UNIQUE(account_id, name)
);

CREATE TABLE messages (
    id              INTEGER PRIMARY KEY,
    folder_id       INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
    uid             INTEGER NOT NULL DEFAULT 0,
    message_id      TEXT,
    thread_id       TEXT,
    subject         TEXT NOT NULL DEFAULT '',
    from_name       TEXT NOT NULL DEFAULT '',
    from_addr       TEXT NOT NULL DEFAULT '',
    to_addrs        TEXT NOT NULL DEFAULT '',
    date            INTEGER NOT NULL DEFAULT 0,
    preview         TEXT NOT NULL DEFAULT '',
    is_read         INTEGER NOT NULL DEFAULT 0,
    is_flagged      INTEGER NOT NULL DEFAULT 0,
    has_attachments INTEGER NOT NULL DEFAULT 0,
    category        TEXT NOT NULL DEFAULT 'primary',
    snoozed_until   INTEGER,
    UNIQUE(folder_id, uid)
);
CREATE INDEX idx_messages_folder_date ON messages(folder_id, date DESC);
CREATE INDEX idx_messages_snoozed ON messages(snoozed_until) WHERE snoozed_until IS NOT NULL;

CREATE TABLE message_bodies (
    message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    html       TEXT,
    text       TEXT
);

CREATE TABLE outbox (
    id         INTEGER PRIMARY KEY,
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    to_addrs   TEXT NOT NULL,
    subject    TEXT NOT NULL DEFAULT '',
    body_text  TEXT,
    body_html  TEXT,
    send_at    INTEGER,
    status     TEXT NOT NULL DEFAULT 'queued',
    error      TEXT,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE TABLE rules (
    id              INTEGER PRIMARY KEY,
    account_id      INTEGER REFERENCES accounts(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    enabled         INTEGER NOT NULL DEFAULT 1,
    conditions_json TEXT NOT NULL DEFAULT '{}',
    actions_json    TEXT NOT NULL DEFAULT '{}',
    position        INTEGER NOT NULL DEFAULT 0
);

CREATE VIRTUAL TABLE messages_fts USING fts5(
    subject, from_name, from_addr, preview,
    content='messages', content_rowid='id'
);

CREATE TRIGGER messages_ai AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts(rowid, subject, from_name, from_addr, preview)
    VALUES (new.id, new.subject, new.from_name, new.from_addr, new.preview);
END;

CREATE TRIGGER messages_ad AFTER DELETE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, subject, from_name, from_addr, preview)
    VALUES ('delete', old.id, old.subject, old.from_name, old.from_addr, old.preview);
END;

CREATE TRIGGER messages_au AFTER UPDATE OF subject, from_name, from_addr, preview ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, subject, from_name, from_addr, preview)
    VALUES ('delete', old.id, old.subject, old.from_name, old.from_addr, old.preview);
    INSERT INTO messages_fts(rowid, subject, from_name, from_addr, preview)
    VALUES (new.id, new.subject, new.from_name, new.from_addr, new.preview);
END;
"#;
