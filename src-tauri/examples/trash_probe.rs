//! Ile naprawdę siedzi w Koszu na serwerze, a ile widzi baza:
//! `cargo run --example trash_probe -- <adres konta>`.
//!
//! Diagnostyka opróżniania Kosza - pokazuje, czy `\Deleted` + EXPUNGE
//! faktycznie coś usunęły, czy serwer je zignorował.

#[tokio::main]
async fn main() {
    let email = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("użycie: cargo run --example trash_probe -- <adres konta>");
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

    let (folder_id, folder_name): (i64, String) = conn
        .query_row(
            "SELECT f.id, f.name FROM folders f JOIN accounts a ON a.id = f.account_id
             WHERE a.email = ?1 AND f.kind = 'trash'",
            [&email],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("folder Kosz");

    let local: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE folder_id = ?1",
            [folder_id],
            |r| r.get(0),
        )
        .expect("licznik");

    let password = lotusmail_lib::accounts::get_password(&email).expect("hasło z keyringu");
    let mut session = lotusmail_lib::sync::connect_session(&host, port, &login, &password)
        .await
        .expect("logowanie IMAP");
    let mailbox = session.select(&folder_name).await.expect("SELECT");

    println!("konto:   {email}");
    println!("folder:  {folder_name}");
    println!("baza:    {local} wiadomości");
    println!("serwer:  {} wiadomości", mailbox.exists);

    // Ile z nich serwer uważa za oznaczone do usunięcia. Jeśli po opróżnianiu
    // jest ich dużo, to znaczy, że `\Deleted` doszło, ale EXPUNGE nie zadziałał.
    let deleted = session.uid_search("DELETED").await.expect("UID SEARCH DELETED");
    println!("oznaczone \\Deleted: {}", deleted.len());

    session.logout().await.ok();
}
