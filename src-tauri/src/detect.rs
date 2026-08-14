//! Automatyczne wykrywanie ustawień serwerów pocztowych dla adresu e-mail.
//!
//! Kolejność prób (jak w Thunderbirdzie):
//! 1. wbudowana lista znanych dostawców,
//! 2. baza autokonfiguracji Thunderbirda + autoconfig na domenie,
//! 3. zgadywanie typowych nazw hostów (imap.domena, mail.domena, …) sondą TCP.

use serde::Serialize;
use std::time::Duration;

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    /// true = STARTTLS na porcie otwartego tekstu; false = bezpośredni TLS
    pub starttls: bool,
}

#[derive(Debug, Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct DetectedConfig {
    pub imap: Option<ServerConfig>,
    pub pop3: Option<ServerConfig>,
    pub smtp: Option<ServerConfig>,
    /// Login do serwera: pełny adres albo sama część lokalna (np. WP).
    pub login: String,
    /// "known" | "autoconfig" | "guess" | "none"
    pub source: String,
}

/// domena, host IMAP, host SMTP (port SMTP), login pełnym adresem?
const KNOWN: &[(&str, &str, &str, u16, bool)] = &[
    ("gmail.com", "imap.gmail.com", "smtp.gmail.com", 465, true),
    ("googlemail.com", "imap.gmail.com", "smtp.gmail.com", 465, true),
    ("outlook.com", "outlook.office365.com", "smtp-mail.outlook.com", 587, true),
    ("hotmail.com", "outlook.office365.com", "smtp-mail.outlook.com", 587, true),
    ("live.com", "outlook.office365.com", "smtp-mail.outlook.com", 587, true),
    ("wp.pl", "imap.wp.pl", "smtp.wp.pl", 465, false),
    ("o2.pl", "poczta.o2.pl", "poczta.o2.pl", 465, false),
    ("interia.pl", "poczta.interia.pl", "poczta.interia.pl", 465, true),
    ("onet.pl", "imap.poczta.onet.pl", "smtp.poczta.onet.pl", 465, true),
    ("op.pl", "imap.poczta.onet.pl", "smtp.poczta.onet.pl", 465, true),
    ("vp.pl", "imap.poczta.onet.pl", "smtp.poczta.onet.pl", 465, true),
    ("gazeta.pl", "imap.gazeta.pl", "smtp.gazeta.pl", 465, true),
    ("yahoo.com", "imap.mail.yahoo.com", "smtp.mail.yahoo.com", 465, true),
    ("icloud.com", "imap.mail.me.com", "smtp.mail.me.com", 587, true),
];

pub async fn detect(email: &str) -> DetectedConfig {
    let Some(domain) = email.split('@').nth(1).map(|d| d.trim().to_ascii_lowercase()) else {
        return DetectedConfig { source: "none".into(), ..Default::default() };
    };
    let localpart = email.split('@').next().unwrap_or_default().to_string();

    // 1. Znani dostawcy
    if let Some((_, imap, smtp, smtp_port, full_login)) =
        KNOWN.iter().find(|(d, ..)| *d == domain)
    {
        return DetectedConfig {
            imap: Some(ServerConfig { host: imap.to_string(), port: 993, starttls: false }),
            pop3: None,
            smtp: Some(ServerConfig { host: smtp.to_string(), port: *smtp_port, starttls: *smtp_port == 587 }),
            login: if *full_login { email.to_string() } else { localpart },
            source: "known".into(),
        };
    }

    // 2. Autokonfiguracja (baza Thunderbirda, potem autoconfig na domenie)
    let urls = [
        format!("https://autoconfig.thunderbird.net/v1.1/{domain}"),
        format!("https://autoconfig.{domain}/mail/config-v1.1.xml?emailaddress={email}"),
        format!("https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml"),
    ];
    if let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(6))
        .build()
    {
        for url in urls {
            let Ok(resp) = client.get(&url).send().await else { continue };
            if !resp.status().is_success() {
                continue;
            }
            let Ok(xml) = resp.text().await else { continue };
            let mut cfg = parse_autoconfig(&xml);
            if cfg.imap.is_some() || cfg.pop3.is_some() {
                cfg.login = resolve_login(&xml, email, &localpart);
                cfg.source = "autoconfig".into();
                return cfg;
            }
        }
    }

    // 3. Zgadywanie typowych hostów sondą TCP
    let imap_candidates = [
        (format!("imap.{domain}"), 993u16),
        (format!("mail.{domain}"), 993),
        (format!("poczta.{domain}"), 993),
    ];
    let smtp_candidates = [
        (format!("smtp.{domain}"), 465u16),
        (format!("smtp.{domain}"), 587),
        (format!("mail.{domain}"), 465),
        (format!("mail.{domain}"), 587),
        (format!("poczta.{domain}"), 465),
    ];
    let mut cfg = DetectedConfig { login: email.to_string(), source: "guess".into(), ..Default::default() };
    for (host, port) in imap_candidates {
        if probe(&host, port).await {
            cfg.imap = Some(ServerConfig { host, port, starttls: false });
            break;
        }
    }
    for (host, port) in smtp_candidates {
        if probe(&host, port).await {
            cfg.smtp = Some(ServerConfig { host: host.clone(), port, starttls: port == 587 });
            break;
        }
    }
    if cfg.imap.is_none() && cfg.smtp.is_none() {
        cfg.source = "none".into();
    }
    cfg
}

async fn probe(host: &str, port: u16) -> bool {
    tokio::time::timeout(
        Duration::from_secs(4),
        tokio::net::TcpStream::connect((host, port)),
    )
    .await
    .map(|r| r.is_ok())
    .unwrap_or(false)
}

/// Autoconfig podaje szablon loginu: %EMAILADDRESS% albo %EMAILLOCALPART%.
fn resolve_login(xml: &str, email: &str, localpart: &str) -> String {
    let section = find_section(xml, "incomingServer", "imap")
        .or_else(|| find_section(xml, "incomingServer", "pop3"))
        .unwrap_or("");
    match tag_value(section, "username").as_deref() {
        Some("%EMAILLOCALPART%") => localpart.to_string(),
        _ => email.to_string(),
    }
}

fn parse_autoconfig(xml: &str) -> DetectedConfig {
    DetectedConfig {
        imap: parse_server(xml, "incomingServer", "imap"),
        pop3: parse_server(xml, "incomingServer", "pop3"),
        smtp: parse_server(xml, "outgoingServer", "smtp"),
        login: String::new(),
        source: String::new(),
    }
}

fn parse_server(xml: &str, element: &str, ty: &str) -> Option<ServerConfig> {
    let section = find_section(xml, element, ty)?;
    let host = tag_value(section, "hostname")?;
    let port: u16 = tag_value(section, "port")?.parse().ok()?;
    let starttls = tag_value(section, "socketType").as_deref() == Some("STARTTLS");
    Some(ServerConfig { host, port, starttls })
}

/// Wycina fragment `<element type="ty"> … </element>` z XML-a autoconfigu.
fn find_section<'a>(xml: &'a str, element: &str, ty: &str) -> Option<&'a str> {
    let marker = format!("type=\"{ty}\"");
    let close = format!("</{element}>");
    let mut search_from = 0;
    while let Some(rel) = xml[search_from..].find(&format!("<{element}")) {
        let start = search_from + rel;
        let end = xml[start..].find(&close).map(|e| start + e)?;
        let section = &xml[start..end];
        if section.lines().next().map(|l| l.contains(&marker)).unwrap_or(false)
            || section[..section.find('>').unwrap_or(section.len())].contains(&marker)
        {
            return Some(section);
        }
        search_from = end + close.len();
    }
    None
}

fn tag_value(section: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = section.find(&open)? + open.len();
    let end = section[start..].find(&close)? + start;
    Some(section[start..end].trim().to_string())
}
