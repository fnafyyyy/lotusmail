//! Bezpieczne przechowywanie haseł kont w pęku kluczy systemu:
//! Menedżer poświadczeń na Windowsie, Keychain na Apple.
//! Hasła nigdy nie trafiają do SQLite.
//!
//! Na iOS omijamy warstwę zgodności `keyring` i rozmawiamy wprost
//! z `keyring_core` - powód opisany przy `entry`.

// AppError potrzebny tylko tam, gdzie sami rejestrujemy magazyn haseł.
#[cfg(target_os = "ios")]
use crate::error::AppError;
use crate::error::Result;

const SERVICE: &str = "pl.pachura.mail-manager";

/// Rejestruje magazyn haseł na iOS.
///
/// Warstwa zgodności `keyring` ustawia magazyn sama, ale **celowo pomija iOS
/// i Androida** - obsługuje wyłącznie macOS, Windows i uniksy. Podpinamy więc
/// chroniony pęk Apple ręcznie, raz na proces, przez `keyring-core`
/// (`keyring` re-eksportuje z niego tylko typy błędów).
///
/// Przyczynę niepowodzenia zapamiętujemy i zwracamy dalej - bez tego wołający
/// dostawał mylące „nie ustawiono magazynu" zamiast prawdziwego powodu.
#[cfg(target_os = "ios")]
fn ensure_store() -> Result<()> {
    static STORE: std::sync::OnceLock<std::result::Result<(), String>> = std::sync::OnceLock::new();
    let outcome = STORE.get_or_init(
        || match apple_native_keyring_store::protected::Store::new() {
            Ok(store) => {
                keyring_core::set_default_store(store);
                Ok(())
            }
            Err(e) => {
                eprintln!("[accounts] pęk kluczy niedostępny: {e}");
                Err(e.to_string())
            }
        },
    );
    match outcome {
        Ok(()) => Ok(()),
        Err(e) => Err(AppError::Other(format!("pęk kluczy niedostępny: {e}"))),
    }
}

/// Wpis w pęku kluczy - na iOS z pominięciem warstwy zgodności.
///
/// `keyring::Entry::new` zaczyna od sprawdzenia własnej inicjalizacji:
///
/// ```ignore
/// if SET_CREDENTIAL_STORE_RESULT.is_err() {
///     return Err(Error::NoDefaultStore);
/// }
/// ```
///
/// Ta inicjalizacja na iOS zawsze zawodzi (platforma nieobsługiwana), więc
/// wpis zostaje odrzucony, **zanim ktokolwiek sprawdzi, czy magazyn jest
/// ustawiony** - nasza rejestracja nie miała żadnego znaczenia. Sięgamy więc
/// wprost do `keyring_core`. Typy błędów są te same, bo `keyring` re-eksportuje
/// je z `keyring_core`.
#[cfg(target_os = "ios")]
fn entry(email: &str) -> Result<keyring_core::Entry> {
    ensure_store()?;
    Ok(keyring_core::Entry::new(SERVICE, email)?)
}

#[cfg(not(target_os = "ios"))]
fn entry(email: &str) -> Result<keyring::Entry> {
    Ok(keyring::Entry::new(SERVICE, email)?)
}

pub fn store_password(email: &str, password: &str) -> Result<()> {
    entry(email)?.set_password(password)?;
    Ok(())
}

#[allow(dead_code)] // używane od fazy 1 (logowanie IMAP/SMTP)
pub fn get_password(email: &str) -> Result<String> {
    Ok(entry(email)?.get_password()?)
}

pub fn delete_password(email: &str) -> Result<()> {
    match entry(email)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.into()),
    }
}
