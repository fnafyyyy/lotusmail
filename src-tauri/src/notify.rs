//! Powiadomienia systemowe o nowej poczcie.
//!
//! Wysyłamy je z rdzenia (nie z interfejsu), żeby działały także wtedy, gdy
//! okno jest zminimalizowane albo w tle - wtedy są najbardziej potrzebne.

use crate::db::Db;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

/// Ile pojedynczych powiadomień pokazujemy, zanim przejdziemy na zbiorcze.
const MAX_SINGLE: usize = 3;

/// Rejestruje aplikację w systemie jako źródło powiadomień. Bez tego Windows
/// podpisuje dymki nazwą procesu, który uruchomił program (w trybie
/// deweloperskim: „PowerShell"), zamiast nazwą i ikoną LotusMaila.
#[cfg(windows)]
pub fn register_app_id(app: &AppHandle) {
    use std::io::Write;
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let identifier = &app.config().identifier;

    // Bez tego Windows przypisuje powiadomienia procesowi, który uruchomił
    // program (w trybie deweloperskim: PowerShell). Zgłaszamy własną tożsamość.
    unsafe {
        let wide: Vec<u16> = identifier.encode_utf16().chain(std::iter::once(0)).collect();
        let _ = windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(
            windows::core::PCWSTR(wide.as_ptr()),
        );
    }

    // Ikona dymka musi być plikiem na dysku - kopiujemy ją obok bazy.
    let icon_path = match app.path().app_data_dir() {
        Ok(dir) => {
            let path = dir.join("icon.png");
            if !path.exists() {
                let _ = std::fs::create_dir_all(&dir);
                if let Ok(mut f) = std::fs::File::create(&path) {
                    let _ = f.write_all(include_bytes!("../icons/128x128.png"));
                }
            }
            path.to_string_lossy().to_string()
        }
        Err(_) => String::new(),
    };

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let path = format!(r"Software\Classes\AppUserModelId\{identifier}");
    if let Ok((key, _)) = hkcu.create_subkey(&path) {
        let _ = key.set_value("DisplayName", &"LotusMail");
        if !icon_path.is_empty() {
            let _ = key.set_value("IconUri", &icon_path);
        }
        let _ = key.set_value("IconBackgroundColor", &"000C1216");
    }
}

#[cfg(not(windows))]
pub fn register_app_id(_app: &AppHandle) {}

pub struct NewMail {
    /// Identyfikator wiadomości w bazie - po kliknięciu dymka otwieramy ją.
    pub id: i64,
    pub from: String,
    pub subject: String,
    pub category: String,
}

fn setting(app: &AppHandle, key: &str, default: bool) -> bool {
    let db = app.state::<Db>();
    let conn = db.0.lock().unwrap();
    conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
        r.get::<_, String>(0)
    })
    .map(|v| v == "1")
    .unwrap_or(default)
}

/// Pokazuje powiadomienia o świeżo pobranych wiadomościach.
pub fn new_mail(app: &AppHandle, mut items: Vec<NewMail>) {
    if items.is_empty() {
        return;
    }
    if !setting(app, "notify", true) {
        eprintln!("[notify] pominięto {} - powiadomienia wyłączone", items.len());
        return;
    }
    if setting(app, "notify_primary_only", false) {
        items.retain(|m| m.category == "primary");
    }
    if items.is_empty() {
        return;
    }

    let total = items.len();
    if total <= MAX_SINGLE {
        for m in items {
            let subject = if m.subject.trim().is_empty() {
                "(bez tematu)".to_string()
            } else {
                m.subject
            };
            show(app, &m.from, &subject, Some(m.id));
        }
    } else {
        // Przy większej paczce jedno zbiorcze - inaczej zasypalibyśmy pulpit.
        let nadawcy: Vec<String> = items
            .drain(..)
            .take(MAX_SINGLE)
            .map(|m| m.from)
            .collect();
        show(
            app,
            &format!("Nowe wiadomości: {total}"),
            &format!("{} i inni", nadawcy.join(", ")),
            None,
        );
    }
}

/// Kliknięcie dymka wysuwa okno na wierzch i otwiera wskazaną wiadomość.
///
/// Wysuwanie okna dotyczy tylko desktopu - na iOS oknami zarządza system
/// (stuknięcie w powiadomienie samo wznawia aplikację), a metody typu
/// `unminimize` nie istnieją tam w ogóle.
fn activate(app: &AppHandle, message_id: Option<i64>) {
    #[cfg(desktop)]
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
    if let Some(id) = message_id {
        let _ = app.emit("open-message", id);
    }
}

/// Na Windowsie wysyłamy dymek sami, z identyfikatorem aplikacji. Wtyczka
/// Tauri celowo go pomija poza wersją instalacyjną, przez co system podpisywał
/// powiadomienia nazwą procesu uruchamiającego (np. „PowerShell").
#[cfg(windows)]
fn show(app: &AppHandle, title: &str, body: &str, message_id: Option<i64>) {
    use tauri_winrt_notification::{Duration, Sound, Toast};

    let handle = app.clone();
    let result = Toast::new(&app.config().identifier)
        .title(title)
        .text1(body)
        .sound(Some(Sound::Default))
        .duration(Duration::Short)
        .on_activated(move |_action| {
            activate(&handle, message_id);
            Ok(())
        })
        .show();
    match result {
        Ok(()) => eprintln!("[notify] dymek: {title} / {body}"),
        Err(e) => {
            eprintln!("[notify] dymek odrzucony ({e}) - próbuję przez wtyczkę");
            if let Err(e) = app.notification().builder().title(title).body(body).show() {
                eprintln!("[notify] wtyczka też nie dała rady: {e}");
            }
        }
    }
}

#[cfg(not(windows))]
fn show(app: &AppHandle, title: &str, body: &str, _message_id: Option<i64>) {
    let _ = app.notification().builder().title(title).body(body).show();
}
