// Publiczne, żeby dało się sprawdzać stan poczty bez uruchamiania okna
// (przykłady w `examples/`, np. `cargo run --example flags_probe`).
pub mod accounts;
mod attachments;
mod commands;
mod db;
mod detect;
mod error;
mod mail;
mod models;
mod notify;
pub mod oauth;
mod outlook;
mod send;
// Publiczny, żeby dało się sprawdzić słownik bez uruchamiania okna:
// `cargo run --example spell_probe`.
pub mod spell;
pub mod sync;
pub mod sync_config;

use std::sync::Mutex;
use tauri::Manager;

/// Wyłącza wbudowane menu prawego przycisku WebView2 (Odśwież, Zapisz jako…).
/// Aplikacja ma własne menu kontekstowe, a w ramkach z treścią maili
/// JavaScript strony nie działa - to jedyne miejsce, gdzie da się je zdjąć.
#[cfg(windows)]
fn disable_webview_context_menu(app: &mut tauri::App) {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings;
    let Some(window) = app.get_webview_window("main") else { return };
    let _ = window.with_webview(|webview| unsafe {
        let core = webview.controller().CoreWebView2();
        if let Ok(core) = core {
            if let Ok(settings) = core.Settings() {
                let settings: ICoreWebView2Settings = settings;
                let _ = settings.SetAreDefaultContextMenusEnabled(false);
            }
        }
    });
}

#[cfg(not(windows))]
fn disable_webview_context_menu(_app: &mut tauri::App) {}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_process::init());

    // Aktualizacje z sieci istnieją tylko na desktopie - na iOS/Androidzie
    // dostarcza je sklep, a wtyczki nie ma w zależnościach dla tych targetów.
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());

    builder
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let conn = db::open(&data_dir.join("mail.db"))
                .map_err(|e| format!("nie udało się otworzyć bazy danych: {e}"))?;
            app.manage(db::Db(Mutex::new(conn)));
            sync::start_scheduler(app.handle().clone());
            notify::register_app_id(app.handle());
            disable_webview_context_menu(app);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_accounts,
            commands::add_account,
            commands::remove_account,
            commands::list_folders,
            commands::list_messages,
            commands::category_counts,
            commands::list_thread,
            commands::get_message,
            commands::list_snoozed,
            commands::get_message_body,
            commands::set_read,
            commands::set_thread_read,
            commands::mark_folder_read,
            commands::set_flagged,
            commands::snooze_message,
            commands::delete_message,
            commands::cleanup_scan,
            commands::cleanup_delete,
            commands::empty_trash,
            commands::sync_set_passphrase,
            commands::sync_has_passphrase,
            commands::sync_export,
            commands::sync_import,
            commands::sync_push,
            commands::sync_pull,
            commands::search_messages,
            commands::search_server,
            commands::queue_send,
            commands::sync_now,
            commands::check_mail,
            commands::detect_settings,
            commands::test_login,
            commands::seed_demo_data,
            commands::get_setting,
            commands::set_setting,
            commands::list_outlook_signatures,
            commands::search_contacts,
            commands::set_sender_name,
            commands::set_account_label,
            commands::create_folder,
            commands::delete_folder,
            commands::reorder_folders,
            commands::get_attachments,
            commands::save_attachment,
            commands::read_attachment,
            commands::list_rules,
            commands::add_rule,
            commands::delete_rule,
            spell::spell_available,
            spell::spell_check,
            spell::spell_suggest,
            spell::spell_add,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
