//! Sprawdza, czy serwer ma w skrzynce odbiorczej wiadomości nowsze niż baza:
//! `cargo run --example newmail_probe -- <adres konta>`.
//!
//! Diagnostyka „nie dochodzą nowe maile" - pokazuje, po której stronie leży
//! problem: czy serwer nic nie ma, czy synchronizacja tego nie zabiera.

#[tokio::main]
async fn main() {
    let email = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("użycie: cargo run --example newmail_probe -- <adres konta>");
        std::process::exit(1);
    });

    let path = std::path::PathBuf::from(std::env::var("APPDATA").unwrap())
        .join("com.pachura.mailmanager")
        .join("mail.db");
    let conn = rusqlite::Connection::open(path).expect("baza");

    let (host, port, login): (String, u16, String) = conn
        .query_row(
            "SELECT imap_host, imap_port, COALESCE(NULLIF(login,''), email)
             FROM accounts WHERE email = ?1",
            [&email],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("konto");

    let (folder_name, last_seen, uid_validity): (String, u32, u32) = conn
        .query_row(
            "SELECT f.name, f.last_seen_uid, f.uid_validity
             FROM folders f JOIN accounts a ON a.id = f.account_id
             WHERE a.email = ?1 AND f.kind = 'inbox'",
            [&email],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("skrzynka odbiorcza");

    let local_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM messages m JOIN folders f ON f.id = m.folder_id
             JOIN accounts a ON a.id = f.account_id
             WHERE a.email = ?1 AND f.kind = 'inbox'",
            [&email],
            |r| r.get(0),
        )
        .expect("licznik");

    let password = lotusmail_lib::accounts::get_password(&email).expect("hasło z keyringu");
    let mut session = lotusmail_lib::sync::connect_session(&host, port, &login, &password)
        .await
        .expect("logowanie IMAP");
    let mailbox = session.select(&folder_name).await.expect("SELECT");

    println!("konto:            {email}");
    println!("folder:           {folder_name}");
    println!("baza:             {local_count} wiadomości, last_seen_uid={last_seen}, uidvalidity={uid_validity}");
    println!(
        "serwer:           {} wiadomości, uidvalidity={:?}, uidnext={:?}",
        mailbox.exists, mailbox.uid_validity, mailbox.uid_next
    );

    // Dokładnie to pytanie zadaje tania ścieżka synchronizacji. Zakres `n:*`
    // zwraca też ostatnią wiadomość, nawet gdy jest starsza - stąd filtr.
    let query = format!("UID {}:*", last_seen + 1);
    let found = session.uid_search(&query).await.expect("UID SEARCH");
    let mut newer: Vec<u32> = found.into_iter().filter(|u| *u > last_seen).collect();
    newer.sort_unstable();

    println!("\nzapytanie:        {query}");
    if newer.is_empty() {
        println!("wynik:            brak wiadomości nowszych niż baza - serwer nic nie ma");
    } else {
        println!("wynik:            {} nowszych UID: {:?}", newer.len(), newer);
    }

    session.logout().await.ok();
}
