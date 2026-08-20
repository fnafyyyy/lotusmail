use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: i64,
    pub email: String,
    pub display_name: String,
    /// Nazwa w nagłówku From wychodzących maili (widoczna u odbiorców).
    pub sender_name: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub auth_kind: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewAccount {
    pub email: String,
    pub display_name: String,
    /// Nazwa nadawcy w polu From (widoczna u odbiorców).
    #[serde(default)]
    pub sender_name: String,
    /// Login do serwera - zwykle pełny adres, u części dostawców sama część lokalna.
    #[serde(default)]
    pub login: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    /// "password" | "oauth2"
    pub auth_kind: String,
    /// Trafia do Menedżera poświadczeń Windows, nigdy do bazy.
    pub password: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: i64,
    pub account_id: i64,
    pub name: String,
    pub display_name: String,
    pub kind: String,
    pub unread_count: i64,
    /// Wszystkie wiadomości folderu (bez odłożonych na drzemkę).
    pub total_count: i64,
}

/// Nieprzeczytane w zakładkach Smart Inbox - liczniki przy przyciskach listy.
#[derive(Debug, Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct CategoryCounts {
    pub primary: i64,
    pub newsletters: i64,
    pub notifications: i64,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MessageSummary {
    pub id: i64,
    pub folder_id: i64,
    pub subject: String,
    pub from_name: String,
    pub from_addr: String,
    pub date: i64,
    pub preview: String,
    pub is_read: bool,
    pub is_flagged: bool,
    pub has_attachments: bool,
    pub category: String,
    pub snoozed_until: Option<i64>,
    /// Klucz konwersacji; `thread_count`/`thread_unread` wypełnione tylko
    /// w widoku listy (pogrupowanym).
    pub thread_id: String,
    pub thread_count: i64,
    pub thread_unread: i64,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MessageBody {
    pub id: i64,
    pub to_addrs: String,
    pub html: Option<String>,
    pub text: Option<String>,
    /// Nagłówki potrzebne przy odpowiadaniu, żeby wątek się skleił.
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
}

/// Reguła: wiadomości od danego nadawcy trafiają do wskazanego folderu.
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    pub id: i64,
    pub account_id: i64,
    pub from_addr: String,
    pub folder_id: i64,
    pub folder_name: String,
    pub enabled: bool,
}

/// Podpowiedź adresata budowana z historii poczty.
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Contact {
    pub addr: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeDraft {
    pub account_id: i64,
    pub to_addrs: String,
    #[serde(default)]
    pub cc_addrs: String,
    /// Ukryta kopia - nie trafia do nagłówków wysyłanej wiadomości, lettre
    /// dokłada tych odbiorców wyłącznie do koperty SMTP.
    #[serde(default)]
    pub bcc_addrs: String,
    /// Message-ID wiadomości, na którą odpowiadamy (bez nawiasów kątowych).
    #[serde(default)]
    pub in_reply_to: Option<String>,
    /// Łańcuch References budowany z wątku - dzięki niemu odpowiedź doklei się
    /// u odbiorcy do właściwej konwersacji, a nie zawiśnie osobno.
    #[serde(default)]
    pub references: Option<String>,
    pub subject: String,
    pub body_text: String,
    /// Sformatowana treść z edytora (wysyłka jako multipart w fazie 3).
    #[serde(default)]
    pub body_html: Option<String>,
    /// Unix epoch - NULL oznacza „wyślij od razu" (Spark: wyślij później).
    pub send_at: Option<i64>,
    #[serde(default)]
    pub attachments: Vec<DraftAttachment>,
}

/// Załącznik szkicu. Treść przechodzi przez most Tauriego w base64, bo JSON
/// nie ma typu binarnego; do bazy trafia już jako BLOB.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DraftAttachment {
    pub filename: String,
    pub mime: String,
    pub size: i64,
    pub data_b64: String,
}

/// Kopia robocza przysłana z edytora. `id` puste = nowy szkic.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftInput {
    pub id: Option<i64>,
    pub account_id: i64,
    pub to_addrs: String,
    pub cc_addrs: String,
    pub bcc_addrs: String,
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
    pub subject: String,
    pub body_html: String,
    #[serde(default)]
    pub is_reply: bool,
    #[serde(default)]
    pub attachments: Vec<DraftAttachment>,
}

/// Kopia robocza odczytana z bazy - to samo, co wyżej, plus czas zapisu.
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StoredDraft {
    pub id: i64,
    pub account_id: i64,
    pub to_addrs: String,
    pub cc_addrs: String,
    pub bcc_addrs: String,
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
    pub subject: String,
    pub body_html: String,
    pub is_reply: bool,
    pub updated_at: i64,
    pub attachments: Vec<DraftAttachment>,
}

/// Grupa kandydatów do sprzątania: wszystko, co przyszło od jednego nadawcy.
/// Sprzątamy nadawcami, a nie pojedynczymi mailami - i tak zwykle chodzi
/// o pozbycie się całej seryjnej korespondencji z jednego źródła.
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CleanupGroup {
    pub from_addr: String,
    pub from_name: String,
    pub count: i64,
    pub unread: i64,
    /// Ile z nich nigdy nie zostało otwartych - najlepsza przesłanka, że
    /// tego nadawcy można skasować w całości.
    pub never_read: i64,
    pub oldest: i64,
    pub newest: i64,
    /// Przeważająca kategoria (newsletters / notifications).
    pub category: String,
    /// Kilka ostatnich tematów - podgląd tego, co poleci.
    pub samples: Vec<String>,
    /// Dokładne identyfikatory do skasowania. Komenda kasująca bierze tę
    /// listę, a nie kryteria wyszukiwania - dzięki temu znika dokładnie to,
    /// co było widoczne na liście, nawet jeśli w międzyczasie coś dojdzie.
    pub ids: Vec<i64>,
}

/// Postęp sprzątania wysyłany eventem `cleanup-progress`.
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CleanupProgress {
    pub done: usize,
    pub total: usize,
}
