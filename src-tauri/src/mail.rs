//! Przetwarzanie wiadomości: sanityzacja HTML, podgląd, kategoryzacja (Smart Inbox).
#![allow(dead_code)] // używane od fazy 1 (silnik synchronizacji IMAP)


/// Czyści HTML maila przed pokazaniem go w UI: wycina skrypty i aktywną
/// treść, ale zostawia obrazki oraz atrybuty tabel, na których opiera się
/// układ typowych maili HTML (newslettery, powiadomienia).
pub fn sanitize_html(html: &str) -> String {
    ammonia::Builder::default()
        .add_generic_attributes(&[
            "style", "width", "height", "align", "valign", "bgcolor", "border",
            "cellpadding", "cellspacing",
            // `id`/`class` są nieaktywne (brak skryptów), ale po nich rozpoznajemy
            // bloki cytowanej historii Outlooka i Gmaila.
            "id", "class", "type",
        ])
        .url_schemes(
            ["http", "https", "mailto", "tel", "data", "cid"]
                .into_iter()
                .collect(),
        )
        .clean(html)
        .to_string()
}

#[cfg(test)]
mod testy_sanitizera {
    #[test]
    fn zachowuje_tresc_maila_z_html_body() {
        let wejscie = "<html><body>Dzień Dobry,<br/><br/>Zgłaszam awarię drukarki.<br/>-- <br/>Aleksander</body></html>";
        let wynik = super::sanitize_html(wejscie);
        println!("WEJSCIE: {wejscie}");
        println!("WYNIK:   {wynik}");
        assert!(wynik.contains("Zgłaszam awarię"), "sanitizer zjadł treść: {wynik}");
    }
}

/// Największy załącznik inline osadzany w treści (2 MB) - powyżej tej granicy
/// data-URI rozdmuchałoby bazę.
const MAX_INLINE_BYTES: usize = 2 * 1024 * 1024;

struct InlinePart {
    cid: Option<String>,
    /// Content-Location lub nazwa pliku - Outlook odnosi się do obrazków
    /// także względną nazwą (np. `src="image001.png"`).
    names: Vec<String>,
    data_uri: String,
}

fn header_text(part: &mail_parser::MessagePart, name: &str) -> Option<String> {
    part.headers
        .iter()
        .find(|h| format!("{}", h.name()).eq_ignore_ascii_case(name))
        .and_then(|h| h.value().as_text())
        .map(|s| s.trim().to_string())
}

/// Podmienia odnośniki do obrazków osadzonych w wiadomości na data-URI, żeby
/// treść w bazie była samowystarczalna mimo braku magazynu załączników.
/// Obsługiwane formy: `cid:…`, względna nazwa pliku (Content-Location) oraz
/// `blob:…` (Outlook mobile zapisuje tak własne wklejki - dopasowujemy
/// po kolei nieużyte obrazki inline).
fn resolve_inline_images(html: &str, msg: &mail_parser::Message) -> String {
    use base64::Engine;
    use mail_parser::MimeHeaders;

    let mut parts: Vec<InlinePart> = Vec::new();
    for part in msg.attachments() {
        let mime = part
            .content_type()
            .map(|ct| match ct.subtype() {
                Some(st) => format!("{}/{}", ct.ctype(), st),
                None => ct.ctype().to_string(),
            })
            .unwrap_or_else(|| "application/octet-stream".to_string());
        if !mime.starts_with("image/") || part.contents().len() > MAX_INLINE_BYTES {
            continue;
        }
        let mut names = Vec::new();
        if let Some(loc) = header_text(part, "Content-Location") {
            names.push(loc);
        }
        if let Some(name) = part.attachment_name() {
            names.push(name.to_string());
        }
        let b64 = base64::engine::general_purpose::STANDARD.encode(part.contents());
        parts.push(InlinePart {
            cid: part
                .content_id()
                .map(|c| c.trim_start_matches('<').trim_end_matches('>').to_string()),
            names,
            data_uri: format!("data:{mime};base64,{b64}"),
        });
    }
    if parts.is_empty() {
        return html.to_string();
    }

    let mut used = vec![false; parts.len()];
    let mut out = String::with_capacity(html.len());
    let mut rest = html;

    while let Some(i) = rest.find("src=\"") {
        let (before, after) = rest.split_at(i + 5);
        out.push_str(before);
        let Some(end) = after.find('"') else {
            out.push_str(after);
            return out;
        };
        let url = &after[..end];
        let resolved = resolve_one(url, &parts, &mut used);
        out.push_str(resolved.unwrap_or(url));
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

fn resolve_one<'a>(url: &str, parts: &'a [InlinePart], used: &mut [bool]) -> Option<&'a str> {
    let pick = |idx: usize, used: &mut [bool]| {
        used[idx] = true;
        Some(parts[idx].data_uri.as_str())
    };

    if let Some(cid) = url.strip_prefix("cid:") {
        let idx = parts
            .iter()
            .position(|p| p.cid.as_deref() == Some(cid))?;
        return pick(idx, used);
    }
    // Już osadzone albo zdalne - zostawiamy bez zmian.
    if url.starts_with("data:") || url.starts_with("http://") || url.starts_with("https://") {
        return None;
    }
    if url.starts_with("blob:") {
        // Nieodtwarzalny odnośnik nadawcy - bierzemy kolejny nieużyty obrazek.
        let idx = used.iter().position(|u| !u)?;
        return pick(idx, used);
    }
    // Odnośnik względny: dopasowanie po Content-Location lub nazwie pliku.
    let tail = url.rsplit('/').next().unwrap_or(url);
    let idx = parts.iter().position(|p| {
        p.names
            .iter()
            .any(|n| n.eq_ignore_ascii_case(url) || n.eq_ignore_ascii_case(tail))
    })?;
    pick(idx, used)
}

/// Skraca tekst do jednolinijkowego podglądu na liście wiadomości.
pub fn make_preview(text: &str, max_chars: usize) -> String {
    let collapsed: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(max_chars).collect()
}

/// Sygnały z nagłówków wykorzystywane przy rozpoznawaniu rodzaju wiadomości.
#[derive(Debug, Default, Clone, Copy)]
pub struct MailSignals {
    /// Link „wypisz się" - znak wysyłki masowej.
    pub list_unsubscribe: bool,
    /// Nagłówek listy dyskusyjnej / wysyłkowej.
    pub list_id: bool,
    /// `Auto-Submitted: auto-generated` - wiadomość wygenerowana przez system.
    pub auto_generated: bool,
    /// `Precedence: bulk|list|junk` - poczta masowa.
    pub bulk: bool,
}

/// Nadawcy, którzy z definicji są automatem: monitoring, kopie zapasowe,
/// systemy zgłoszeń, demony pocztowe. Dopasowanie po części przed małpą
/// albo po domenie - `starts_with` przepuszczało `backup@`, `jira@` czy `root@`.
const AUTOMATION_MARKERS: &[&str] = &[
    "no-reply", "noreply", "no_reply", "do-not-reply", "donotreply", "notification",
    "notifications", "mailer-daemon", "postmaster", "bounce", "alerts", "alert",
    "backup", "monitoring", "monitor", "nagios", "zabbix", "cron", "daemon",
    "jenkins", "jira", "confluence", "gitlab", "github", "sonar", "root",
    "automat", "system", "raport", "report",
];

const AUTOMATION_DOMAINS: &[&str] = &["atlassian.net", "statuspage.io", "veeam.com"];

/// Rozpoznanie zakładki Smart Inbox: "primary" | "newsletters" | "notifications".
///
/// Kolejność ma znaczenie: wiadomości systemowe rozpoznajemy przed masowymi,
/// bo raporty z Jiry czy kopii zapasowych też bywają wysyłane z linkiem
/// „wypisz się", a mimo to nie są newsletterem.
pub fn categorize(from_addr: &str, signals: MailSignals) -> &'static str {
    let addr = from_addr.to_ascii_lowercase();
    let local = addr.split('@').next().unwrap_or(&addr);
    let domain = addr.split('@').nth(1).unwrap_or("");

    let automated = AUTOMATION_MARKERS.iter().any(|m| local.contains(m))
        || AUTOMATION_DOMAINS.iter().any(|d| domain.ends_with(d));

    if signals.auto_generated || automated {
        return "notifications";
    }
    if signals.list_unsubscribe || signals.list_id || signals.bulk {
        return "newsletters";
    }
    "primary"
}

/// Temat bez przedrostków odpowiedzi/przekazania - klucz grupowania rozmów
/// dla wiadomości bez nagłówków References.
/// Czy temat wygląda na odpowiedź albo przekazanie dalej.
///
/// Rozstrzyga o wątkowaniu: wiadomość bez takiego przedrostka zaczyna własny
/// wątek, nawet jeśli ktoś już kiedyś wysłał coś o tym samym tytule. Bez tego
/// każdy kolejny mail o temacie „test" doklejał się do poprzedniego i po
/// miesiącu w jednej „konwersacji" siedziały wiadomości do różnych osób.
pub fn is_reply_subject(subject: &str) -> bool {
    let lower = subject.trim().to_lowercase();
    REPLY_PREFIXES.iter().any(|p| lower.starts_with(*p))
}

const REPLY_PREFIXES: &[&str] = &["re:", "odp:", "fwd:", "fw:", "pd:", "podaj dalej:"];

pub fn subject_key(subject: &str) -> String {
    const PREFIXES: &[&str] = REPLY_PREFIXES;
    let mut s = subject.trim();
    loop {
        let lower = s.to_lowercase();
        match PREFIXES.iter().find(|p| lower.starts_with(**p)) {
            Some(p) => s = s[p.len()..].trim_start(),
            None => break,
        }
    }
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Wynik parsowania surowej wiadomości RFC 5322 - wypełniany przez silnik
/// synchronizacji (faza 1), tutaj żeby cały przepływ miał już swój typ.
#[derive(Debug)]
pub struct ParsedEmail {
    pub message_id: Option<String>,
    /// Identyfikatory z In-Reply-To i References - wiążą wiadomość z wątkiem.
    pub refs: Vec<String>,
    pub subject: String,
    pub from_name: String,
    pub from_addr: String,
    pub to_addrs: String,
    pub date: i64,
    pub html: Option<String>,
    pub text: Option<String>,
    pub has_attachments: bool,
    pub category: &'static str,
}

/// Wyciąga listę identyfikatorów wiadomości z nagłówka (In-Reply-To/References).
fn header_ids(value: &mail_parser::HeaderValue) -> Vec<String> {
    let norm = |s: &str| s.trim().trim_start_matches('<').trim_end_matches('>').to_string();
    match value {
        mail_parser::HeaderValue::Text(t) => vec![norm(t)],
        mail_parser::HeaderValue::TextList(list) => list.iter().map(|s| norm(s)).collect(),
        _ => Vec::new(),
    }
}

/// Parsuje surowe bajty wiadomości (MIME) do postaci gotowej do zapisu w bazie.
pub fn parse_email(raw: &[u8]) -> Option<ParsedEmail> {
    let msg = mail_parser::MessageParser::default().parse(raw)?;

    let (from_name, from_addr) = msg
        .from()
        .and_then(|a| a.first())
        .map(|addr| {
            (
                addr.name().unwrap_or_default().to_string(),
                addr.address().unwrap_or_default().to_string(),
            )
        })
        .unwrap_or_default();

    let to_addrs = msg
        .to()
        .map(|a| {
            a.iter()
                .filter_map(|addr| addr.address())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();

    // HTML trafia do bazy surowy - sanityzacja dzieje się przy wyświetlaniu
    // (get_message_body), więc zmiany polityki nie wymagają ponownego pobrania.
    // Obrazki inline (cid:) osadzamy od razu jako data-URI, bo załączników
    // nie przechowujemy osobno.
    // Część maili (np. kalendarzowe albo z zagnieżdżonym multipart) nie ma
    // części HTML/tekstowej w slocie 0 - bierzemy pierwszą dostępną.
    let html_raw = msg
        .body_html(0)
        .or_else(|| msg.html_bodies().next().and_then(|p| p.text_contents().map(Into::into)))
        .map(|h| h.to_string());
    let html = html_raw.map(|h| {
        if h.contains("src=") { resolve_inline_images(&h, &msg) } else { h }
    });
    let text = msg
        .body_text(0)
        .map(|t| t.to_string())
        .or_else(|| msg.text_bodies().next().and_then(|p| p.text_contents().map(|s| s.to_string())));
    let header_text = |name: &str| -> String {
        msg.header(name)
            .and_then(|v| v.as_text())
            .unwrap_or_default()
            .to_ascii_lowercase()
    };
    let auto_submitted = header_text("Auto-Submitted");
    let precedence = header_text("Precedence");
    let signals = MailSignals {
        list_unsubscribe: msg.header("List-Unsubscribe").is_some(),
        list_id: msg.header("List-Id").is_some(),
        auto_generated: auto_submitted.starts_with("auto-generated")
            || auto_submitted.starts_with("auto-replied"),
        bulk: matches!(precedence.trim(), "bulk" | "list" | "junk"),
    };
    let from_addr_for_cat = from_addr.clone();

    let mut refs = header_ids(msg.in_reply_to());
    refs.extend(header_ids(msg.references()));

    Some(ParsedEmail {
        message_id: msg.message_id().map(|s| s.to_string()),
        refs,
        subject: msg.subject().unwrap_or_default().to_string(),
        from_name,
        from_addr,
        to_addrs,
        date: msg.date().map(|d| d.to_timestamp()).unwrap_or(0),
        has_attachments: msg.attachment_count() > 0,
        category: categorize(&from_addr_for_cat, signals),
        html,
        text,
    })
}
