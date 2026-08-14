//! Porównuje stan „przeczytane" w lokalnej bazie z tym, co mówi serwer IMAP.
//! Diagnostyka rozjazdów liczników: `cargo run --example flags_probe -- <email>`.

use futures_util::TryStreamExt;

#[tokio::main]
async fn main() {
    let email = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("użycie: cargo run --example flags_probe -- <adres konta>");
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
             WHERE a.email = ?1 AND f.kind = 'inbox'",
            [&email],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("skrzynka odbiorcza");

    let password = lotusmail_lib::accounts::get_password(&email).expect("hasło z keyringu");
    let mut session = lotusmail_lib::sync::connect_session(&host, port, &login, &password)
        .await
        .expect("logowanie IMAP");
    session.select(&folder_name).await.expect("SELECT");

    let server_unseen = session.uid_search("UNSEEN").await.expect("UID SEARCH UNSEEN");

    let mut local_unread: Vec<u32> = {
        let mut stmt = conn
            .prepare("SELECT uid FROM messages WHERE folder_id = ?1 AND is_read = 0 AND uid > 0")
            .unwrap();
        let rows = stmt.query_map([folder_id], |r| r.get(0)).unwrap();
        rows.filter_map(|r| r.ok()).collect()
    };
    local_unread.sort_unstable();

    println!("konto: {email}  folder: {folder_name}");
    println!("nieprzeczytane wg serwera: {}", server_unseen.len());
    println!("nieprzeczytane w bazie:    {}", local_unread.len());

    let local_set: std::collections::HashSet<u32> = local_unread.iter().copied().collect();
    let tylko_serwer: Vec<_> = server_unseen.difference(&local_set).collect();
    let tylko_baza: Vec<u32> = local_set.difference(&server_unseen).copied().collect();
    println!("tylko na serwerze: {}", tylko_serwer.len());
    println!("tylko w bazie:     {}", tylko_baza.len());

    // Dla rozjazdów pokazujemy tematy - łatwiej rozpoznać, o co chodzi.
    for uid in tylko_baza.iter().take(10) {
        let subject: String = conn
            .query_row(
                "SELECT substr(subject,1,50) FROM messages WHERE folder_id = ?1 AND uid = ?2",
                rusqlite::params![folder_id, uid],
                |r| r.get(0),
            )
            .unwrap_or_default();
        println!("  baza mówi nieprzeczytany, serwer nie: uid={uid} {subject}");
    }

    // Flagi prosto z serwera dla kilku spornych UID-ów.
    if !tylko_baza.is_empty() {
        let set = tylko_baza
            .iter()
            .take(5)
            .map(|u| u.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let stream = session.uid_fetch(set, "(UID FLAGS)").await.expect("FETCH");
        let items = stream.try_collect::<Vec<_>>().await.expect("FETCH");
        for f in &items {
            println!("  uid={:?} flagi={:?}", f.uid, f.flags().collect::<Vec<_>>());
        }
    }

    session.logout().await.ok();
}
